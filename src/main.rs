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

/// Helper to configure the gsettings command securely, 
/// injecting the local schema path if running in development mode.
fn build_gsettings_cmd() -> Command {
    let mut cmd = Command::new("gsettings");
    
    // Inject schema dir for local development via ./run.sh
    if Path::new("schemas").exists() {
        cmd.env("GSETTINGS_SCHEMA_DIR", "schemas");
    } else if let Ok(home) = env::var("HOME") {
        // Fallback for user-local installations
        let ext_schema = format!("{}/.local/share/gnome-shell/extensions/gnome-lens/schemas", home);
        if Path::new(&ext_schema).exists() {
            cmd.env("GSETTINGS_SCHEMA_DIR", ext_schema);
        }
    }
    
    cmd
}

fn get_gsettings_bool(key: &str) -> bool {
    if let Ok(output) = build_gsettings_cmd()
        .arg("get")
        .arg("org.gnome.shell.extensions.gnome-lens")
        .arg(key)
        .output()
    {
        if !output.status.success() {
            eprintln!("[GSettings Error] Failed to read {}: {}", key, String::from_utf8_lossy(&output.stderr).trim());
        }
        return String::from_utf8_lossy(&output.stdout).trim() == "true";
    }
    false
}

fn get_gsettings_int(key: &str, default: usize) -> usize {
    if let Ok(output) = build_gsettings_cmd()
        .arg("get")
        .arg("org.gnome.shell.extensions.gnome-lens")
        .arg(key)
        .output()
    {
        if !output.status.success() {
            eprintln!("[GSettings Error] Failed to read {}: {}", key, String::from_utf8_lossy(&output.stderr).trim());
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let parts: Vec<&str> = stdout.split_whitespace().collect();
        if let Some(last) = parts.last() {
            if let Ok(val) = last.parse::<usize>() {
                return val;
            }
        }
    }
    default
}

fn get_gsettings_array(key: &str) -> Vec<String> {
    if let Ok(output) = build_gsettings_cmd()
        .arg("get")
        .arg("org.gnome.shell.extensions.gnome-lens")
        .arg(key)
        .output()
    {
        if !output.status.success() {
            eprintln!("[GSettings Error] Failed to read {}: {}", key, String::from_utf8_lossy(&output.stderr).trim());
            return Vec::new();
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        println!("[Boot DEBUG] Raw gsettings output for {}: {:?}", key, stdout);
        
        let cleaned = stdout
            .replace("[", "")
            .replace("]", "")
            .replace("'", "")
            .replace("\"", "")
            .replace("@as", "");
            
        let mut results = Vec::new();
        for s in cleaned.split(',') {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                results.push(trimmed.to_string());
            }
        }
        println!("[Boot DEBUG] Parsed gsettings array for {}: {:?}", key, results);
        return results;
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

    let max_depth = get_gsettings_int("index-max-depth", 3);

    if args.len() > 1 {
        let command = &args[1];

        if command == "index" || command == "reindex" {
            let vector_store = Arc::new(VectorStore::new(&db_path));
            
            if command == "reindex" {
                println!("Force re-index requested. Resetting all database timestamps...");
                vector_store.force_reindex_all();
            }

            if let Some(target_dir) = args.get(2) {
                let blacklist = get_gsettings_array("index-blacklist");
                println!("Triggering manual recursive ingestion for: {}", target_dir);
                let pipeline = IngestionPipeline::new(Arc::clone(&vector_store), &config_dir, blacklist);
                pipeline.run_indexer(vec![target_dir.clone()], max_depth);
            } else {
                eprintln!("Error: Please provide a directory path. Usage: gnome-lens {} /path/to/dir", command);
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

    if max_depth > 3 {
        println!("\n===================================================================");
        println!("[WARNING] Max recursion depth is set to {} (Default is 3).", max_depth);
        println!("Linux has a strict kernel limit on inotify watches (fs.inotify.max_user_watches).");
        println!("If the daemon fails to watch all files, you MUST increase this OS limit:");
        println!("Execute: echo 'fs.inotify.max_user_watches=524288' | sudo tee -a /etc/sysctl.conf && sudo sysctl -p");
        println!("===================================================================\n");
    }

    let vector_store = Arc::new(VectorStore::new(&db_path));
    
    let is_full_system = get_gsettings_bool("index-full-system");
    let blacklist = get_gsettings_array("index-blacklist");
    let mut target_directories = Vec::new();

    if is_full_system {
        println!("[Boot] Full System Indexation Enabled.");
        target_directories.push(home_dir.clone());
    } else {
        println!("[Boot] Custom Path Indexation Enabled.");
        let mut user_paths = get_gsettings_array("index-paths");
        
        if user_paths.is_empty() {
            println!("[Boot Warning] No user paths retrieved from gsettings. Defaulting to home directory (~).");
            user_paths.push("~".to_string());
        }

        for p in user_paths {
            let expanded_path = p.replace("~", &home_dir);
            target_directories.push(expanded_path);
        }
        target_directories.push("/usr/share/applications".to_string());
        target_directories.push("/etc".to_string());
    }

    println!("[Boot DEBUG] Target directories before validation: {:?}", target_directories);
    
    target_directories.retain(|dir| {
        let exists = Path::new(dir).exists();
        if !exists {
            println!("[Boot Warning] Dropping directory because it does not exist on disk: {}", dir);
        }
        exists
    });
    
    println!("[Boot DEBUG] Target directories after validation (passed to watcher): {:?}", target_directories);

    let pipeline = Arc::new(IngestionPipeline::new(Arc::clone(&vector_store), &config_dir, blacklist));

    // STARTUP RECONCILIATION
    let initial_pipeline = Arc::clone(&pipeline);
    let target_dirs_clone = target_directories.clone();
    thread::spawn(move || {
        initial_pipeline.run_indexer(target_dirs_clone, max_depth); 
    });

    // MARK AND SWEEP GARBAGE COLLECTION
    let gc_store = Arc::clone(&vector_store);
    thread::spawn(move || {
        // Wait 5 minutes to let the CPU settle after the boot-up ingestion sweep
        // This ensures the user experiences no lag when first logging in.
        thread::sleep(std::time::Duration::from_secs(300));
        println!("[Daemon] Running background Garbage Collection sweep...");
        gc_store.prune_orphans();
    });

    let index_triggers: Vec<Box<dyn IndexTrigger>> = vec![
        Box::new(INotifyTrigger),
    ];

    println!("Loading Gnome Lens Triggers:");
    for trigger in &index_triggers {
        println!("  ✓ {}", trigger.name());
        trigger.start(target_directories.clone(), max_depth, Arc::clone(&pipeline));
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