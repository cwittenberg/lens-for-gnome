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
use std::env;
use std::thread;
use std::time::Instant;

use crate::vector::VectorStore;
use crate::ingestion::IngestionPipeline;
use crate::plugins::{MathPlugin, EmailPlugin, VectorSearchPlugin, PluginTool};
use crate::engine::{SystemRouter, ThreadPool};
use crate::triggers::{INotifyTrigger, IndexTrigger};

fn handle_client(mut stream: UnixStream, router: Arc<SystemRouter>) {
    let mut buffer = [0; 4096];
    if let Ok(bytes_read) = stream.read(&mut buffer) {
        if bytes_read > 0 {
            let request = String::from_utf8_lossy(&buffer[..bytes_read]);
            
            let start_time = Instant::now();
            
            // Prevent visual confusion in the logs by distinguishing 
            // internal IPC payloads from actual text queries.
            if request.trim().starts_with("{\"action\":") {
                println!("[Daemon] Received IPC Command: {}", request.trim());
            } else {
                println!("[Daemon] Received Search Query: {}", request.trim());
            }
            
            router.handle_request(&request, |chunk| {
                let mut payload = chunk.clone();
                payload.push('\n'); 
                let _ = stream.write_all(payload.as_bytes());
                let _ = stream.flush(); 
            });

            println!("[Daemon] Finished streaming response in {:.2?}", start_time.elapsed());
        }
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    
    // ==========================================
    // SECURE PATH RESOLUTION (XDG BASE DIRECTORY)
    // ==========================================
    let home_dir = env::var("HOME").expect("HOME environment variable must be set");
    
    // Config Directory for Dynamic Domains
    let config_dir = format!("{}/.config/gnome-lens", home_dir);
    if !Path::new(&config_dir).exists() {
        fs::create_dir_all(&config_dir).expect("Failed to create secure config directory");
    }

    // Data Directory for SQLite
    let data_dir = format!("{}/.local/share/gnome-lens", home_dir);
    if !Path::new(&data_dir).exists() {
        fs::create_dir_all(&data_dir).expect("Failed to create secure data directory");
    }
    let db_path = format!("{}/gnome-lens.db", data_dir);

    // State Directory for Unix Socket
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
                println!("Triggering manual recursive ingestion for: {}", target_dir);
                let pipeline = IngestionPipeline::new(Arc::clone(&vector_store), &config_dir);
                pipeline.run_indexer(target_dir);
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
                        if bytes == 0 {
                            break; 
                        }
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

    // ==========================================
    // DAEMON BOOTSTRAPPER
    // ==========================================
    
    let vector_store = Arc::new(VectorStore::new(&db_path));
    let target_directory = format!("{}/Documents", home_dir);
    
    let pipeline = Arc::new(IngestionPipeline::new(Arc::clone(&vector_store), &config_dir));

    let initial_pipeline = Arc::clone(&pipeline);
    let target_dir_clone = target_directory.to_string();
    thread::spawn(move || {
        initial_pipeline.run_indexer(&target_dir_clone); 
    });

    let index_triggers: Vec<Box<dyn IndexTrigger>> = vec![
        Box::new(INotifyTrigger),
    ];

    println!("Loading Gnome Lens Triggers:");
    for trigger in &index_triggers {
        println!("  ✓ {}", trigger.name());
        trigger.start(target_directory.to_string(), Arc::clone(&pipeline));
    }

    let plugins: Vec<Box<dyn PluginTool>> = vec![
        Box::new(MathPlugin),
        Box::new(EmailPlugin),
        Box::new(VectorSearchPlugin::new(Arc::clone(&vector_store))),
    ];

    println!("Loading Gnome Lens Plugins:");
    for plugin in &plugins {
        println!("  ✓ {} [{}]", plugin.name(), plugin.id());
    }

    let router = Arc::new(SystemRouter::new(plugins, &config_dir));

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