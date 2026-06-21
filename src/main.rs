// src/main.rs
mod domain;
mod vector;
mod ingestion;
mod plugins;
mod engine;
mod triggers;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::env;
use std::thread;
use std::time::Instant;
use std::process::Command;

use crate::vector::VectorStore;
use crate::ingestion::IngestionPipeline;
use crate::plugins::{MathPlugin, EmailPlugin, AppLauncherPlugin, VectorSearchPlugin, PluginTool};
use crate::engine::{SystemRouter, ThreadPool};
use crate::triggers::{INotifyTrigger, IndexTrigger};

fn get_gsettings_bool(key: &str) -> bool {
    if let Ok(output) = Command::new("gsettings")
        .arg("get")
        .arg("org.gnome.shell.extensions.gnome-lens")
        .arg(key)
        .output()
    {
        return String::from_utf8_lossy(&output.stdout).trim() == "true";
    }
    false
}

fn get_gsettings_array(key: &str) -> Vec<String> {
    if let Ok(output) = Command::new("gsettings")
        .arg("get")
        .arg("org.gnome.shell.extensions.gnome-lens")
        .arg(key)
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Clean the gsettings array format: ['item1', 'item2'] -> item1, item2
        let cleaned = stdout.replace("['", "").replace("']", "").replace('\'', "");
        if cleaned.trim().is_empty() || cleaned.trim() == "[]" {
            return Vec::new();
        }
        return cleaned.split(", ").map(|s| s.to_string()).collect();
    }
    Vec::new()
}

fn handle_client(mut stream: UnixStream, router: Arc<SystemRouter>) {
    let mut buffer = [0; 4096];
    if let Ok(bytes_read) = stream.read(&mut buffer) {
        if bytes_read > 0 {
            let request = String::from_utf8_lossy(&buffer[..bytes_read]);
            
            if request.contains("\"cancel\"") {
                return;
            }

            let is_cancelled = Arc::new(AtomicBool::new(false));
            let cancel_flag = Arc::clone(&is_cancelled);
            
            if let Ok(mut stream_clone) = stream.try_clone() {
                thread::spawn(move || {
                    let mut buf = [0; 128];
                    while let Ok(n) = stream_clone.read(&mut buf) {
                        if n == 0 {
                            cancel_flag.store(true, Ordering::Relaxed);
                            break;
                        }
                        if String::from_utf8_lossy(&buf[..n]).contains("\"cancel\"") {
                            cancel_flag.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                });
            }
            
            let start_time = Instant::now();
            
            if request.trim().starts_with("{\"action\":") {
                println!("[Daemon] Received IPC Command: {}", request.trim());
            } else {
                println!("[Daemon] Received Search Query: {}", request.trim());
            }
            
            router.handle_request(&request, is_cancelled, |chunk| {
                let mut payload = chunk.clone();
                payload.push('\n'); 
                let _ = stream.write_all(payload.as_bytes());
                let _ = stream.flush(); 
            });

            println!("[Daemon] Finished processing stream in {:.2?}", start_time.elapsed());
        }
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let home_dir = env::var("HOME").expect("HOME environment variable must be set");
    
    let config_dir = format!("{}/.config/gnome-lens", home_dir);
    if !Path::new(&config_dir).exists() {
        fs::create_dir_all(&config_dir).expect("Failed to create secure config directory");
    }

    let data_dir = format!("{}/.local/share/gnome-lens", home_dir);
    if !Path::new(&data_dir).exists() {
        fs::create_dir_all(&data_dir).expect("Failed to create secure data directory");
    }
    let db_path = format!("{}/gnome-lens.db", data_dir);

    let state_dir = format!("{}/.local/state/gnome-lens", home_dir);
    if !Path::new(&state_dir).exists() {
        fs::create_dir_all(&state_dir).expect("Failed to create secure state directory");
    }
    let socket_path = format!("{}/gnome_lens.sock", state_dir);

    if args.len() > 1 {
        let command = &args[1];

        if command == "index" {
            let vector_store = Arc::new(VectorStore::new(&db_path));
            if let Some(target_dir) = args.get(2) {
                let blacklist = get_gsettings_array("index-blacklist");
                println!("Triggering manual recursive ingestion for: {}", target_dir);
                let pipeline = IngestionPipeline::new(Arc::clone(&vector_store), &config_dir, blacklist);
                pipeline.run_indexer(vec![target_dir.clone()]);
            } else {
                eprintln!("Error: Please provide a directory path. Usage: gnome-lens index /path/to/dir");
            }
            return Ok(());
        } else {
            let query_text = args[1..].join(" ");
            let payload = serde_json::json!({ "query": query_text }).to_string();
            
            if let Ok(mut stream) = UnixStream::connect(&socket_path) {
                if stream.write_all(payload.as_bytes()).is_ok() {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    
                    while let Ok(bytes) = reader.read_line(&mut line) {
                        if bytes == 0 { break; }
                        print!("{}", line);
                        line.clear();
                    }
                }
            } else {
                eprintln!("Error: Could not connect to the daemon at {}. Is the background service running?", socket_path);
            }
            return Ok(());
        }
    }

    let vector_store = Arc::new(VectorStore::new(&db_path));
    
    // Dynamically build the target pools from User Prefs via GSettings
    let is_full_system = get_gsettings_bool("index-full-system");
    let blacklist = get_gsettings_array("index-blacklist");
    let mut target_directories = Vec::new();

    if is_full_system {
        println!("[Boot] Full System Indexation Enabled.");
        target_directories.push(home_dir.clone());
    } else {
        println!("[Boot] Custom Path Indexation Enabled.");
        let user_paths = get_gsettings_array("index-paths");
        for p in user_paths {
            let expanded_path = p.replace("~", &home_dir);
            target_directories.push(expanded_path);
        }
        // Always include these absolute basics for the Universal App Launcher to work
        target_directories.push("/usr/share/applications".to_string());
        target_directories.push("/etc".to_string());
    }

    target_directories.retain(|dir| Path::new(dir).exists());

    let pipeline = Arc::new(IngestionPipeline::new(Arc::clone(&vector_store), &config_dir, blacklist));

    let initial_pipeline = Arc::clone(&pipeline);
    let target_dirs_clone = target_directories.clone();
    thread::spawn(move || {
        initial_pipeline.run_indexer(target_dirs_clone); 
    });

    let index_triggers: Vec<Box<dyn IndexTrigger>> = vec![
        Box::new(INotifyTrigger),
    ];

    println!("Loading Gnome Lens Triggers:");
    for trigger in &index_triggers {
        println!("  ✓ {}", trigger.name());
        trigger.start(target_directories.clone(), Arc::clone(&pipeline));
    }

    let plugins: Vec<Box<dyn PluginTool>> = vec![
        Box::new(MathPlugin),
        Box::new(EmailPlugin),
        Box::new(AppLauncherPlugin::new()),
        Box::new(VectorSearchPlugin::new(Arc::clone(&vector_store))),
    ];

    println!("Loading Gnome Lens Plugins:");
    for plugin in &plugins {
        println!("  ✓ {} [{}]", plugin.name(), plugin.id());
    }

    // Pass the active vector store to the router so the AST engine can natively read metadata
    let router = Arc::new(SystemRouter::new(plugins, Arc::clone(&vector_store), &config_dir));

    if Path::new(&socket_path).exists() {
        fs::remove_file(&socket_path)?;
    }
    let listener = UnixListener::bind(&socket_path)?;
    
    let mut perms = fs::metadata(&socket_path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&socket_path, perms)?;

    let pool = ThreadPool::new(4);

    println!("Gnome Lens Daemon running securely on {}", socket_path);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let router_clone = Arc::clone(&router);
                pool.execute(move || {
                    handle_client(stream, router_clone);
                });
            }
            Err(err) => eprintln!("Socket connection error: {}", err),
        }
    }

    Ok(())
}