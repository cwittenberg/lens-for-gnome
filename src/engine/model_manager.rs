// src/engine/model_manager.rs
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

pub struct ModelManager;

impl ModelManager {
    /// Bootstraps the application configuration and handles default deployment.
    ///
    /// Existing user configs are preserved:
    /// - Existing active_model is kept if it still points to a known model.
    /// - Existing model entries are not overwritten.
    /// - Missing default models are added automatically.
    /// - Missing fields inside existing default model entries are filled in.
    pub fn setup_model_config() -> Value {
        let home = env::var("HOME").expect("HOME environment variable must be set");
        let config_dir = format!("{}/.config/lens-for-gnome", home);
        let models_dir = format!("{}/.local/share/lens-for-gnome/models", home);

        fs::create_dir_all(&config_dir).expect("Failed to create config directory");
        fs::create_dir_all(&models_dir).expect("Failed to create models directory");

        let config_path = format!("{}/models.json", config_dir);
        let default_json = Self::default_model_config();

        if !Path::new(&config_path).exists() {
            fs::write(
                &config_path,
                serde_json::to_string_pretty(&default_json).expect("Failed to serialize default models.json"),
            )
            .expect("Failed to write default models.json");

            return default_json;
        }

        let parsed_config = match fs::read_to_string(&config_path) {
            Ok(content) => serde_json::from_str::<Value>(&content).unwrap_or_else(|_| default_json.clone()),
            Err(_) => default_json.clone(),
        };

        let (merged_config, changed) = Self::merge_with_default_model_config(parsed_config, &default_json);

        if changed {
            fs::write(
                &config_path,
                serde_json::to_string_pretty(&merged_config).expect("Failed to serialize merged models.json"),
            )
            .expect("Failed to write merged models.json");
        }

        merged_config
    }

    /// Fetches the config and dynamically injects the `is_installed` status for the frontend.
    pub fn get_full_config() -> Value {
        let mut config = Self::setup_model_config();
        let home = env::var("HOME").unwrap_or_default();
        
        if let Some(models) = config["models"].as_object_mut() {
            for (_, model) in models.iter_mut() {
                if let Some(filename) = model["filename"].as_str() {
                    let path = format!("{}/.local/share/lens-for-gnome/models/{}", home, filename);
                    model["is_installed"] = json!(Self::model_file_exists(&path));
                } else {
                    model["is_installed"] = json!(false);
                }
            }
        }
        
        config
    }

    /// Resolves the absolute path, URL, and architecture flags of the currently active model.
    pub fn get_active_model_details() -> (String, String, bool) {
        let parsed_config = Self::setup_model_config();
        let fallback_config = Self::default_model_config();

        let active_key = parsed_config["active_model"]
            .as_str()
            .or_else(|| fallback_config["active_model"].as_str())
            .unwrap_or("qwen-2.5-3b");

        let model_obj = parsed_config["models"]
            .get(active_key)
            .or_else(|| fallback_config["models"].get(active_key))
            .or_else(|| fallback_config["models"].get("qwen-2.5-3b"))
            .expect("Default model configuration is missing qwen-2.5-3b");

        let filename = model_obj["filename"]
            .as_str()
            .unwrap_or("qwen2.5-3b-instruct-q4_k_m.gguf");

        let url = model_obj["url"]
            .as_str()
            .unwrap_or("https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf");

        // Gracefully clean markdown formatted links from legacy configs
        let mut safe_url = url.to_string();
        if safe_url.starts_with('[') && safe_url.contains("](") {
            if let Some(idx) = safe_url.find("](") {
                safe_url = safe_url[idx + 2..].trim_end_matches(')').to_string();
            }
        }

        let supports_cot = model_obj["supports_cot"]
            .as_bool()
            .unwrap_or_else(|| active_key.to_lowercase().contains("qwen"));

        let home = env::var("HOME").expect("HOME environment variable must be set");
        let model_path = format!("{}/.local/share/lens-for-gnome/models/{}", home, filename);

        (model_path, safe_url, supports_cot)
    }

    /// Validates the model exists on disk, falling back to a blocking sync download.
    pub fn ensure_model_available(model_path: &str, url: &str) {
        if Self::model_file_exists(model_path) {
            return;
        }

        println!("\n=======================================================");
        println!("Local AI Model not found at: {}", model_path);
        println!("Downloading model from: {}", url);
        println!("This may take several minutes depending on your connection.");
        println!("=======================================================\n");

        if let Err(err) = Self::download_file_blocking(model_path, url) {
            panic!("{}", err);
        }
    }

    /// Dynamic async-like downloader that parses cURL output and pipes it to the GNOME UI socket.
    pub fn download_model_if_needed<F>(
        model_id: &str,
        send_chunk: &mut F,
        is_cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>
    ) -> Result<String, String>
    where
        F: FnMut(String),
    {
        let parsed = Self::setup_model_config();

        let model_obj = parsed["models"]
            .get(model_id)
            .ok_or_else(|| format!("Model ID not found in configuration: {}", model_id))?;

        let filename = model_obj["filename"]
            .as_str()
            .ok_or_else(|| format!("Model '{}' is missing required field: filename", model_id))?;

        let url = model_obj["url"]
            .as_str()
            .ok_or_else(|| format!("Model '{}' is missing required field: url", model_id))?;

        // Gracefully clean markdown formatted links from legacy configs
        let mut safe_url = url.to_string();
        if safe_url.starts_with('[') && safe_url.contains("](") {
            if let Some(idx) = safe_url.find("](") {
                safe_url = safe_url[idx + 2..].trim_end_matches(')').to_string();
            }
        }

        let home = env::var("HOME").map_err(|_| "HOME environment variable must be set".to_string())?;
        let model_path = format!("{}/.local/share/lens-for-gnome/models/{}", home, filename);

        if Self::model_file_exists(&model_path) {
            return Ok(model_path);
        }

        if let Some(parent) = Path::new(&model_path).parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create model directory: {}", e))?;
        }

        let temp_model_path = format!("{}.download", model_path);
        let _ = fs::remove_file(&temp_model_path);

        send_chunk(json!({
            "status": "processing",
            "message": "Connecting to model repository..."
        }).to_string());

        let mut child = Command::new("curl")
            .arg("-L")
            .arg("--fail")
            .arg("-#")
            .arg("-o")
            .arg(&temp_model_path)
            .arg(&safe_url)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start curl: {}", e))?;

        if let Some(stderr) = child.stderr.take() {
            let mut last_reported = -1;
            let mut current_line = String::new();

            for byte in stderr.bytes() {
                // Check if the user cancelled the download via UI
                if is_cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = fs::remove_file(&temp_model_path);
                    return Err("Download cancelled by user.".to_string());
                }

                let b = match byte {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                print!("{}", b as char);
                let _ = std::io::stdout().flush();

                if b == b'\r' || b == b'\n' {
                    if let Some(percent) = Self::parse_curl_progress_percent(&current_line) {
                        // Fix for HuggingFace HTTP 302 Redirects which hit 100% on the redirect payload 
                        // instantly, blocking subsequent updates for the real file payload.
                        if percent < last_reported && last_reported >= 95 && percent <= 5 {
                            last_reported = -1;
                        }

                        if percent > last_reported {
                            send_chunk(json!({
                                "status": "downloading",
                                "progress": percent,
                                "message": format!("Downloading model ({}%)...", percent)
                            }).to_string());

                            last_reported = percent;
                        }
                    }

                    current_line.clear();
                } else {
                    current_line.push(b as char);
                }
            }

            if let Some(percent) = Self::parse_curl_progress_percent(&current_line) {
                if percent > last_reported {
                    send_chunk(json!({
                        "status": "downloading",
                        "progress": percent,
                        "message": format!("Downloading model ({}%)...", percent)
                    }).to_string());
                }
            }
        }

        let status = child
            .wait()
            .map_err(|_| "Download process failed to wait".to_string())?;

        if !status.success() {
            let _ = fs::remove_file(&temp_model_path);
            return Err("Download failed. Check internet connection or model URL.".to_string());
        }

        fs::rename(&temp_model_path, &model_path)
            .map_err(|e| {
                let _ = fs::remove_file(&temp_model_path);
                format!("Failed to finalize downloaded model: {}", e)
            })?;

        send_chunk(json!({
            "status": "processing",
            "message": "Model download completed."
        }).to_string());

        Ok(model_path)
    }

    /// Persists the active model selection to the config block.
    pub fn set_active_model(model_id: &str) -> Result<(), String> {
        let home = env::var("HOME").map_err(|_| "HOME environment variable must be set".to_string())?;
        let config_path = format!("{}/.config/lens-for-gnome/models.json", home);

        let mut parsed = Self::setup_model_config();

        if parsed["models"].get(model_id).is_none() {
            return Err(format!("Model ID not found in configuration: {}", model_id));
        }

        parsed["active_model"] = json!(model_id);

        fs::write(
            &config_path,
            serde_json::to_string_pretty(&parsed).map_err(|_| "Failed to serialize models.json".to_string())?,
        )
        .map_err(|_| "Failed to write updated models.json".to_string())?;

        Ok(())
    }

    /// Deletes the local GGUF file from the disk.
    pub fn delete_model(model_id: &str) -> Result<(), String> {
        let parsed = Self::setup_model_config();
        
        let active_model = parsed["active_model"].as_str().unwrap_or("");
        if model_id == active_model {
            return Err("Cannot delete the currently active model.".to_string());
        }

        let model_obj = parsed["models"].get(model_id)
            .ok_or_else(|| "Model not found in configuration.".to_string())?;
        
        let filename = model_obj["filename"].as_str()
            .ok_or_else(|| "Missing filename for model.".to_string())?;
        
        let home = env::var("HOME").unwrap_or_default();
        let path = format!("{}/.local/share/lens-for-gnome/models/{}", home, filename);
        
        if Path::new(&path).exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    fn default_model_config() -> Value {
        json!({
            "active_model": "qwen3-4b-q4-k-m",
            "models": {
                "qwen-2.5-3b": {
                    "name": "Qwen 2.5 (3B)",
                    "filename": "qwen2.5-3b-instruct-q4_k_m.gguf",
                    "url": "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
                    "size_gb": 1.9,
                    "ram_required_gb": 2.8,
                    "parameters": "3.0B",
                    "context_tokens": 32768,
                    "category": "fastest",
                    "recommended": true,
                    "supports_cot": true,
                    "description": "Fastest useful local model in this list. Good for quick local responses, translation, summarization, OCR cleanup, and small helper tasks."
                },
                "qwen3-4b-q4-k-m": {
                    "name": "Qwen 3 (4B)",
                    "filename": "Qwen3-4B-Q4_K_M.gguf",
                    "url": "https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf",
                    "size_gb": 2.6,
                    "ram_required_gb": 4.5,
                    "parameters": "4.0B",
                    "context_tokens": 32768,
                    "category": "recommended-default",
                    "recommended": true,
                    "supports_cot": true,
                    "description": "Best fast and still accurate default model for this machine. Noticeably faster than 7B/8B models while staying much smarter than tiny 1B/2B models."
                },             
                "nemotron-mini-4b-q4-k-m": {
                    "name": "NVIDIA Nemotron Mini 4B Instruct",
                    "filename": "Nemotron-Mini-4B-Instruct-Q4_K_M.gguf",
                    "url": "https://huggingface.co/bartowski/Nemotron-Mini-4B-Instruct-GGUF/resolve/main/Nemotron-Mini-4B-Instruct-Q4_K_M.gguf",
                    "size_gb": 2.6,
                    "ram_required_gb": 4.0,
                    "parameters": "4.0B",
                    "context_tokens": 4096,
                    "category": "fast-reasoning",
                    "recommended": true,
                    "supports_cot": false,
                    "description": "Fast, compact SLM optimized for RAG and function calling. Runs exceptionally well on Vulkan."
                },
                "qwen2.5-coder-7b-q4-k-m": {
                    "name": "Qwen 2.5 Coder (7B)",
                    "filename": "Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf",
                    "url": "https://huggingface.co/bartowski/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf",
                    "size_gb": 4.68,
                    "ram_required_gb": 7.0,
                    "parameters": "7.0B",
                    "context_tokens": 32768,
                    "category": "coding",
                    "recommended": true,
                    "supports_cot": true,
                    "description": "Fast coding-specialized model. Best practical local coding choice when speed still matters."
                },
                "qwen3-8b-q4-k-m": {
                    "name": "Qwen 3 (8B)",
                    "filename": "Qwen3-8B-Q4_K_M.gguf",
                    "url": "https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf",
                    "size_gb": 4.7,
                    "ram_required_gb": 7.0,
                    "parameters": "8.2B",
                    "context_tokens": 32768,
                    "category": "balanced-general",
                    "recommended": true,
                    "supports_cot": true,
                    "description": "Stronger general-purpose model than Qwen3 4B. Good when quality matters more than raw speed."
                },
                "qwen2.5-coder-14b-q4-k-m": {
                    "name": "Qwen 2.5 Coder (14B)",
                    "filename": "qwen2.5-coder-14b-instruct-q4_k_m.gguf",
                    "url": "https://huggingface.co/Qwen/Qwen2.5-Coder-14B-Instruct-GGUF/resolve/main/qwen2.5-coder-14b-instruct-q4_k_m.gguf",
                    "size_gb": 8.9,
                    "ram_required_gb": 12.0,
                    "parameters": "14.7B",
                    "context_tokens": 32768,
                    "category": "serious-coding",
                    "recommended": true,
                    "supports_cot": true,
                    "description": "Serious local coding model. Slower than 7B, but much stronger for code reasoning and multi-step fixes."
                },
                "qwen3-14b-q4-k-m": {
                    "name": "Qwen 3 (14B)",
                    "filename": "Qwen3-14B-Q4_K_M.gguf",
                    "url": "https://huggingface.co/bartowski/Qwen_Qwen3-14B-GGUF/resolve/main/Qwen3-14B-Q4_K_M.gguf",
                    "size_gb": 9.0,
                    "ram_required_gb": 13.0,
                    "parameters": "14.8B",
                    "context_tokens": 32768,
                    "category": "large-general",
                    "recommended": true,
                    "supports_cot": true,
                    "description": "Higher-quality Qwen3 option that still fits comfortably in 32 GB RAM. Good for reasoning when speed matters less."
                },
                "qwen3-coder-30b-a3b-ud-q4-k-xl": {
                    "name": "Qwen 3 Coder 30B-A3B",
                    "filename": "Qwen3-Coder-30B-A3B-Instruct-UD-Q4_K_XL.gguf",
                    "url": "https://huggingface.co/unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF/resolve/main/Qwen3-Coder-30B-A3B-Instruct-UD-Q4_K_XL.gguf",
                    "size_gb": 17.7,
                    "ram_required_gb": 24.0,
                    "parameters": "30.5B total / 3.3B active",
                    "context_tokens": 32768,
                    "category": "heavy-coding",
                    "recommended": false,
                    "supports_cot": true,
                    "description": "High-end local coding model. Realistic on a 32 GB machine, but heavy; expect slower startup and inference."
                },
                "devstral-small-2507-q4-k-m": {
                    "name": "Devstral Small 2507",
                    "filename": "Devstral-Small-2507-Q4_K_M.gguf",
                    "url": "https://huggingface.co/mistralai/Devstral-Small-2507_gguf/resolve/main/Devstral-Small-2507-Q4_K_M.gguf",
                    "size_gb": 14.33,
                    "ram_required_gb": 22.0,
                    "parameters": "24B",
                    "context_tokens": 131072,
                    "category": "agentic-coding",
                    "recommended": false,
                    "supports_cot": false,
                    "description": "Agentic software-engineering model. Fits in 32 GB RAM at Q4_K_M, but it is heavy and better suited to long coding-agent sessions."
                }
            }
        })
    }

    fn merge_with_default_model_config(mut current: Value, defaults: &Value) -> (Value, bool) {
        let mut changed = false;

        if !current.is_object() {
            return (defaults.clone(), true);
        }

        if current.get("active_model").and_then(Value::as_str).is_none() {
            current["active_model"] = defaults["active_model"].clone();
            changed = true;
        }

        if current.get("models").and_then(Value::as_object).is_none() {
            current["models"] = defaults["models"].clone();
            changed = true;
        }

        if let (Some(current_models), Some(default_models)) = (
            current.get_mut("models").and_then(Value::as_object_mut),
            defaults.get("models").and_then(Value::as_object),
        ) {
            for (model_id, default_model) in default_models {
                if !current_models.contains_key(model_id) {
                    current_models.insert(model_id.clone(), default_model.clone());
                    changed = true;
                    continue;
                }

                if let Some(existing_model) = current_models.get_mut(model_id) {
                    if let (Some(existing_fields), Some(default_fields)) = (
                        existing_model.as_object_mut(),
                        default_model.as_object(),
                    ) {
                        for (field_name, default_field_value) in default_fields {
                            if !existing_fields.contains_key(field_name) {
                                existing_fields.insert(field_name.clone(), default_field_value.clone());
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        let active_model_id = current["active_model"]
            .as_str()
            .unwrap_or("qwen-2.5-3b")
            .to_string();

        let active_model_exists = current["models"].get(&active_model_id).is_some();

        if !active_model_exists {
            current["active_model"] = defaults["active_model"].clone();
            changed = true;
        }

        (current, changed)
    }

    fn model_file_exists(model_path: &str) -> bool {
        fs::metadata(model_path)
            .map(|metadata| metadata.is_file() && metadata.len() > 0)
            .unwrap_or(false)
    }

    fn download_file_blocking(model_path: &str, url: &str) -> Result<(), String> {
        if let Some(parent) = Path::new(model_path).parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create model directory: {}", e))?;
        }

        let temp_model_path = format!("{}.download", model_path);
        let _ = fs::remove_file(&temp_model_path);

        let status = Command::new("curl")
            .arg("-L")
            .arg("--fail")
            .arg("-#")
            .arg("-o")
            .arg(&temp_model_path)
            .arg(url)
            .status()
            .map_err(|e| format!("Failed to execute curl to download the model: {}", e))?;

        if !status.success() {
            let _ = fs::remove_file(&temp_model_path);
            return Err("Failed to download the model. Please check your internet connection or model URL.".to_string());
        }

        fs::rename(&temp_model_path, model_path)
            .map_err(|e| {
                let _ = fs::remove_file(&temp_model_path);
                format!("Failed to finalize downloaded model: {}", e)
            })?;

        Ok(())
    }

    fn parse_curl_progress_percent(line: &str) -> Option<i32> {
        for token in line.split_whitespace().rev() {
            let cleaned = token
                .trim()
                .trim_end_matches('%')
                .trim_matches('#')
                .trim_matches('-')
                .trim_matches('=');

            if cleaned.is_empty() {
                continue;
            }

            if let Ok(value) = cleaned.parse::<f32>() {
                if (0.0..=100.0).contains(&value) {
                    return Some(value.round() as i32);
                }
            }
        }

        None
    }
}