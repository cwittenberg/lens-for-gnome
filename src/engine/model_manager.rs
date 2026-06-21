// src/engine/model_manager.rs
use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::io::Read;
use serde_json::{json, Value};

pub struct ModelManager;

impl ModelManager {
    /// Bootstraps the application configuration and handles default deployment
    pub fn setup_model_config() -> Value {
        let home = env::var("HOME").expect("HOME environment variable must be set");
        let config_dir = format!("{}/.config/gnome-lens", home);
        let models_dir = format!("{}/.local/share/gnome-lens/models", home);
        
        fs::create_dir_all(&config_dir).expect("Failed to create config directory");
        fs::create_dir_all(&models_dir).expect("Failed to create models directory");

        let config_path = format!("{}/models.json", config_dir);
        let default_json = json!({
            "active_model": "phi-3-mini-4k",
            "models": {
                "phi-3-mini-4k": {
                    "name": "Phi 3 Mini (4K)",
                    "filename": "Phi-3-mini-4k-instruct-q4.gguf",
                    "url": "https://huggingface.co/microsoft/Phi-3-mini-4k-instruct-gguf/resolve/main/Phi-3-mini-4k-instruct-q4.gguf",
                    "size_gb": 2.4,
                    "ram_required_gb": 3.0,
                    "parameters": "3.8B",
                    "description": "Microsoft's highly capable small model. Fast and highly reliable default."
                },
                "llama-3.1-8b": {
                    "name": "Llama 3.1 (8B)",
                    "filename": "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
                    "url": "https://huggingface.co/bartowski/Meta-Llama-3.1-8B-Instruct-GGUF/resolve/main/Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
                    "size_gb": 4.9,
                    "ram_required_gb": 6.0,
                    "parameters": "8.0B",
                    "description": "Meta's industry standard. Exceptionally smart for RAG but uses more RAM."
                },
                "qwen-2.5-3b": {
                    "name": "Qwen 2.5 (3B)",
                    "filename": "qwen2.5-3b-instruct-q4_k_m.gguf",
                    "url": "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
                    "size_gb": 1.9,
                    "ram_required_gb": 2.5,
                    "parameters": "3.0B",
                    "description": "Ultra-lightweight and lightning fast. Punches way above its weight class."
                },
                "qwen-2.5-7b": {
                    "name": "Qwen 2.5 (7B)",
                    "filename": "qwen2.5-7b-instruct-q4_k_m.gguf",
                    "url": "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m.gguf",
                    "size_gb": 4.5,
                    "ram_required_gb": 5.5,
                    "parameters": "7.0B",
                    "description": "Outstanding multilingual support for non-English queries."
                }
            }
        });

        if !Path::new(&config_path).exists() {
            fs::write(&config_path, serde_json::to_string_pretty(&default_json).unwrap())
                .expect("Failed to write default models.json");
        }

        let content = fs::read_to_string(&config_path).unwrap_or_else(|_| default_json.to_string());
        serde_json::from_str(&content).unwrap_or(default_json)
    }

    /// Resolves the absolute path and URL of the currently active model
    pub fn get_active_model_path_and_url() -> (String, String) {
        let parsed_config = Self::setup_model_config();
        let active_key = parsed_config["active_model"].as_str().unwrap_or("phi-3-mini-4k");
        let model_obj = &parsed_config["models"][active_key];
        
        let filename = model_obj["filename"].as_str().unwrap_or("Phi-3-mini-4k-instruct-q4.gguf");
        let url = model_obj["url"].as_str().unwrap_or("https://huggingface.co/microsoft/Phi-3-mini-4k-instruct-gguf/resolve/main/Phi-3-mini-4k-instruct-q4.gguf");

        let home = env::var("HOME").unwrap();
        let model_path = format!("{}/.local/share/gnome-lens/models/{}", home, filename);
        
        (model_path, url.to_string())
    }

    /// Validates the model exists on disk, falling back to a blocking sync download
    pub fn ensure_model_available(model_path: &str, url: &str) {
        if !Path::new(model_path).exists() {
            println!("\n=======================================================");
            println!("Local AI Model not found at: {}", model_path);
            println!("Downloading model from: {}", url);
            println!("This may take several minutes depending on your connection.");
            println!("=======================================================\n");
            
            let status = Command::new("curl")
                .arg("-L")
                .arg("-#") 
                .arg("-o")
                .arg(model_path)
                .arg(url)
                .status()
                .expect("Failed to execute curl to download the model");
                
            if !status.success() {
                let _ = fs::remove_file(model_path);
                panic!("Failed to download the model. Please check your internet connection.");
            }
        }
    }

    /// Dynamic async-like downloader that parses cURL output and pipes it to the GNOME UI socket
    pub fn download_model_if_needed<F>(
        model_id: &str,
        send_chunk: &mut F
    ) -> Result<String, String> 
    where F: FnMut(String) 
    {
        let home = env::var("HOME").unwrap();
        let config_path = format!("{}/.config/gnome-lens/models.json", home);
        let content = fs::read_to_string(&config_path).map_err(|_| "Failed to read models.json")?;
        let parsed: Value = serde_json::from_str(&content).map_err(|_| "Invalid models.json format")?;
        
        let model_obj = parsed["models"].get(model_id).ok_or("Model ID not found in configuration")?;
        let filename = model_obj["filename"].as_str().unwrap();
        let url = model_obj["url"].as_str().unwrap();
        
        let model_path = format!("{}/.local/share/gnome-lens/models/{}", home, filename);

        if !Path::new(&model_path).exists() {
            send_chunk(serde_json::json!({"status": "processing", "message": "Connecting to mirror..."}).to_string());
            
            let mut child = Command::new("curl")
                .args(["-L", "-#", "-o", &model_path, url])
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("Failed to start curl: {}", e))?;

            if let Some(stderr) = child.stderr.take() {
                let mut last_reported = -1;
                let mut current_line = String::new();
                
                for byte in stderr.bytes() {
                    if let Ok(b) = byte {
                        if b == b'\r' || b == b'\n' {
                            let trimmed = current_line.trim();
                            if trimmed.ends_with('%') {
                                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                                if let Some(last) = parts.last() {
                                    if let Ok(val) = last.trim_end_matches('%').parse::<f32>() {
                                        let p_int = val as i32;
                                        if p_int > last_reported && p_int % 2 == 0 {
                                            send_chunk(serde_json::json!({
                                                "status": "processing", 
                                                "message": format!("Downloading model ({}%)...", p_int)
                                            }).to_string());
                                            last_reported = p_int;
                                        }
                                    }
                                }
                            }
                            current_line.clear();
                        } else {
                            current_line.push(b as char);
                        }
                    }
                }
            }
            
            let status = child.wait().map_err(|_| "Download process failed to wait")?;
            if !status.success() {
                let _ = fs::remove_file(&model_path);
                return Err("Download failed. Check internet connection.".to_string());
            }
        }

        Ok(model_path)
    }

    /// Persists the active model selection to the config block
    pub fn set_active_model(model_id: &str) -> Result<(), String> {
        let home = env::var("HOME").unwrap();
        let config_path = format!("{}/.config/gnome-lens/models.json", home);
        let content = fs::read_to_string(&config_path).map_err(|_| "Failed to read models.json")?;
        let mut parsed: Value = serde_json::from_str(&content).map_err(|_| "Invalid models.json format")?;
        
        parsed["active_model"] = serde_json::json!(model_id);
        fs::write(&config_path, serde_json::to_string_pretty(&parsed).unwrap())
            .map_err(|_| "Failed to write updated models.json".to_string())?;

        Ok(())
    }
}