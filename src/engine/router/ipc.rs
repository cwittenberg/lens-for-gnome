// src/engine/router/ipc.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use crate::engine::llm::LlmService;
use crate::engine::vision::VisionEngine;
use crate::engine::HardwareManager;
use crate::engine::model_manager::ModelManager;
use crate::vector::VectorStore;

pub fn handle_ipc_action<F>(
    json: &serde_json::Value,
    llm: &Arc<LlmService>,
    vision: &Arc<VisionEngine>,
    store: &Arc<VectorStore>,
    is_cancelled: Arc<AtomicBool>,
    req_start: Instant,
    send_chunk: &mut F
) -> bool
where
    F: FnMut(String),
{
    // ACTION: Health Ping (EGO Compliant Status Check)
    if json["action"].as_str() == Some("ping") {
        send_chunk(serde_json::json!({"status": "pong"}).to_string());
        return true;
    }

    // ACTION: Graceful Shutdown (EGO Compliant Termination)
    if json["action"].as_str() == Some("shutdown") {
        send_chunk(serde_json::json!({
            "status": "done",
            "message": "Shutting down daemon gracefully..."
        }).to_string());
        
        // Give the socket a moment to flush the response to the UI before exiting
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            std::process::exit(0);
        });
        return true;
    }

    // ACTION: Return Database Statistics
    if json["action"].as_str() == Some("get_db_stats") {
        let (records, size_bytes) = store.get_db_stats();
        send_chunk(serde_json::json!({
            "status": "db_stats",
            "records": records,
            "size_bytes": size_bytes
        }).to_string());
        return true;
    }

    // ACTION: Trigger Database Re-index
    if json["action"].as_str() == Some("reindex") {
        store.force_reindex_all();
        send_chunk(serde_json::json!({
            "status": "done",
            "message": "Database timestamps reset. Re-indexing will occur on the next sweep or file change."
        }).to_string());
        return true;
    }

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
            let filepath = json["filepath"].as_str().unwrap_or("");
            
            let mut clean_exec = exec.replace("%u", "")
                .replace("%U", "")
                .replace("%f", "")
                .replace("%F", "")
                .replace("%c", "");

            if !filepath.is_empty() {
                clean_exec = format!("{} \"{}\"", clean_exec.trim(), filepath);
            }

            let mut parts = clean_exec.split_whitespace();
            if let Some(cmd) = parts.next() {
                let args: Vec<&str> = parts.collect();
                
                println!("[IPC Chain] Stage 1: Backend attempting direct execution of '{}'", cmd);
                
                let spawn_res = std::process::Command::new(cmd)
                    .args(&args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();

                match spawn_res {
                    Ok(_) => {
                        println!("[IPC Chain] Stage 2 (Success): Application spawned directly by daemon.");
                        send_chunk(serde_json::json!({
                            "status": "done",
                            "message": format!("Launched {}", cmd)
                        }).to_string());
                    },
                    Err(e) => {
                        println!("[IPC Chain] Stage 2 (Fail): Direct execution failed ({}).", e);
                        if !filepath.is_empty() {
                            println!("[IPC Chain] Stage 3: Delegating to GNOME UI fallback...");
                            send_chunk(serde_json::json!({
                                "status": "delegate",
                                "action": "open_file",
                                "path": filepath,
                                "message": "Specific app launch failed. Delegating to GNOME fallback."
                            }).to_string());
                            return true;
                        }
                        
                        println!("[IPC Chain] Stage 3: No fallback filepath provided. Aborting.");
                        send_chunk(serde_json::json!({
                            "status": "error",
                            "message": format!("Failed to execute or find command: {}", cmd)
                        }).to_string());
                    }
                }
            }
        }
        return true;
    }

    // ACTION: Universal File Launcher
    if json["action"].as_str() == Some("open_file") {
        if let Some(path) = json["path"].as_str() {
            println!("[IPC Chain] Stage 1: Backend attempting to open file natively (gio/xdg-open): {}", path);
            
            // Use .status() to block and get the exit code, rather than detaching with .spawn()
            let status_res = std::process::Command::new("gio")
                .arg("open")
                .arg(path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .or_else(|_| {
                    std::process::Command::new("xdg-open")
                        .arg(path)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                });

            let success = match status_res {
                Ok(exit_status) => exit_status.success(),
                Err(_) => false,
            };

            if success {
                println!("[IPC Chain] Stage 2 (Success): File opened via backend.");
                send_chunk(serde_json::json!({
                    "status": "done",
                    "message": "File opened via native OS handler."
                }).to_string());
            } else {
                println!("[IPC Chain] Stage 2 (Fail): Backend open failed. Delegating to GNOME UI fallback...");
                send_chunk(serde_json::json!({
                    "status": "delegate",
                    "action": "open_file",
                    "path": path,
                    "message": "Backend execution failed, delegating to GNOME Shell."
                }).to_string());
            }
        }
        return true;
    }

    // ACTION: Open Folder Location
    if json["action"].as_str() == Some("open_folder") {
        if let Some(path_str) = json["path"].as_str() {
            let path = std::path::Path::new(path_str);
            if let Some(parent) = path.parent() {
                println!("[IPC Chain] Stage 1: Backend attempting to open folder natively: {:?}", parent);
                
                let status_res = std::process::Command::new("gio")
                    .arg("open")
                    .arg(parent)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .or_else(|_| {
                        std::process::Command::new("xdg-open")
                            .arg(parent)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status()
                    });

                let success = match status_res {
                    Ok(exit_status) => exit_status.success(),
                    Err(_) => false,
                };

                if success {
                    println!("[IPC Chain] Stage 2 (Success): Folder opened via backend.");
                    send_chunk(serde_json::json!({
                        "status": "done",
                        "message": "Folder location opened via native OS handler."
                    }).to_string());
                } else {
                    println!("[IPC Chain] Stage 2 (Fail): Backend folder open failed. Delegating to GNOME UI...");
                    send_chunk(serde_json::json!({
                        "status": "delegate",
                        "action": "open_folder",
                        "path": path_str,
                        "message": "Backend execution failed, delegating to GNOME Shell."
                    }).to_string());
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