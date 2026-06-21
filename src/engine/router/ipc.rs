// src/engine/router/ipc.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use crate::engine::llm::LlmService;
use crate::engine::vision::VisionEngine;
use crate::engine::HardwareManager;
use crate::engine::model_manager::ModelManager;

pub fn handle_ipc_action<F>(
    json: &serde_json::Value,
    llm: &Arc<LlmService>,
    vision: &Arc<VisionEngine>,
    is_cancelled: Arc<AtomicBool>,
    req_start: Instant,
    send_chunk: &mut F
) -> bool
where
    F: FnMut(String),
{
    if json["action"].as_str() == Some("get_config") {
        let parsed_config = ModelManager::get_full_config();
        
        send_chunk(serde_json::json!({
            "status": "config_data",
            "data": parsed_config
        }).to_string());
        return true;
    }

    if json["action"].as_str() == Some("get_hardware_status") {
        let hw_status = HardwareManager::detect_hardware();
        send_chunk(serde_json::json!({
            "status": "hardware_data",
            "data": hw_status
        }).to_string());
        return true;
    }

    if json["action"].as_str() == Some("delete_model") {
        if let Some(model_id) = json["model_id"].as_str() {
            match ModelManager::delete_model(model_id) {
                Ok(_) => {
                    send_chunk(serde_json::json!({
                        "status": "done",
                        "message": "Model deleted successfully."
                    }).to_string());
                },
                Err(e) => {
                    send_chunk(serde_json::json!({
                        "status": "error",
                        "message": e
                    }).to_string());
                }
            }
        }
        return true;
    }

    if json["action"].as_str() == Some("update_config") {
        if let Some(key) = json["key"].as_str() {
            let value = &json["value"];
            
            if key == "active_model" {
                if let Some(model_id) = value.as_str() {
                    send_chunk(serde_json::json!({"status": "processing", "message": "Initiating model switch..."}).to_string());
                    
                    match llm.switch_model(model_id, send_chunk, Arc::clone(&is_cancelled)) {
                        Ok(_) => {
                            send_chunk(serde_json::json!({
                                "status": "done",
                                "message": "Model swapped successfully."
                            }).to_string());
                        },
                        Err(e) => {
                            send_chunk(serde_json::json!({
                                "status": "error",
                                "message": e
                            }).to_string());
                        }
                    }
                }
            }
        }
        return true;
    }
    
    // ACTION: App Launcher
    if json["action"].as_str() == Some("launch_app") {
        if let Some(exec) = json["exec"].as_str() {
            let clean_exec = exec.replace("%u", "")
                .replace("%U", "")
                .replace("%f", "")
                .replace("%F", "")
                .replace("%c", "");

            let mut parts = clean_exec.split_whitespace();
            if let Some(cmd) = parts.next() {
                let args: Vec<&str> = parts.collect();
                
                let spawn_res = std::process::Command::new(cmd)
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();

                match spawn_res {
                    Ok(_) => {
                        send_chunk(serde_json::json!({
                            "status": "done",
                            "message": format!("Launched {}", cmd)
                        }).to_string());
                    },
                    Err(e) => {
                        send_chunk(serde_json::json!({
                            "status": "error",
                            "message": format!("Failed to launch: {}", e)
                        }).to_string());
                    }
                }
            }
        }
        return true;
    }

    // ACTION: Universal File Launcher (mlocate integration)
    if json["action"].as_str() == Some("open_file") {
        if let Some(path) = json["path"].as_str() {
            let spawn_res = std::process::Command::new("xdg-open")
                .arg(path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();

            match spawn_res {
                Ok(_) => {
                    send_chunk(serde_json::json!({
                        "status": "done",
                        "message": "File opened via native OS handler."
                    }).to_string());
                },
                Err(e) => {
                    send_chunk(serde_json::json!({
                        "status": "error",
                        "message": format!("Failed to open file: {}", e)
                    }).to_string());
                }
            }
        }
        return true;
    }

    // ACTION: Open Folder Location
    if json["action"].as_str() == Some("open_folder") {
        if let Some(path_str) = json["path"].as_str() {
            let path = std::path::Path::new(path_str);
            if let Some(parent) = path.parent() {
                let spawn_res = std::process::Command::new("xdg-open")
                    .arg(parent)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();

                match spawn_res {
                    Ok(_) => {
                        send_chunk(serde_json::json!({
                            "status": "done",
                            "message": "Folder location opened via native OS handler."
                        }).to_string());
                    },
                    Err(e) => {
                        send_chunk(serde_json::json!({
                            "status": "error",
                            "message": format!("Failed to open folder: {}", e)
                        }).to_string());
                    }
                }
            } else {
                send_chunk(serde_json::json!({
                    "status": "error",
                    "message": "Could not determine parent directory."
                }).to_string());
            }
        }
        return true;
    }

    // PHASE 1: Intercept Vision IPC Requests
    if json["action"].as_str() == Some("extract_snip") {
        if let Some(path) = json["path"].as_str() {
            send_chunk(serde_json::json!({"status": "processing", "message": "Analyzing image..."}).to_string());
            let result = vision.process_image(path);
            send_chunk(serde_json::json!({
                "status": "final",
                "mode": "vision_extraction",
                "results": result
            }).to_string());
        } else {
            send_chunk(r#"{"status": "error", "message": "Missing path for extraction"}"#.to_string());
        }
        println!("[Router DEBUG] Vision extraction completed in {:.2?}", req_start.elapsed());
        return true;
    }

    false
}