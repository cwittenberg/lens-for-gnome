// src/triggers/inotify_watcher.rs

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use walkdir::WalkDir;

use crate::ingestion::IngestionPipeline;
use super::IndexTrigger;

pub struct INotifyTrigger;

const EVENT_DEBOUNCE: Duration = Duration::from_millis(500);
const RX_IDLE_TICK: Duration = Duration::from_millis(100);

enum WatchMessage {
    WatchDir(PathBuf),
    Done(String),
}

/// Checks if the immediate file or folder is hidden.
/// Explicitly ignores GTK's temporary save files to prevent massive parent directory re-walks.
fn is_hidden(path: &Path) -> bool {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    
    if file_name.starts_with(".goutputstream") || file_name.ends_with(".tmp") {
        return true;
    }
    
    // FIX: We must only check the explicit filename for a leading dot. 
    // Iterating over path.components() causes the kernel to falsely ignore 
    // EVERYTHING inside the `~/.local/...` or `~/.config/...` directories!
    file_name.starts_with('.') && file_name != "." && file_name != ".."
}

/// Calculates depth relative to the configured base watch directories.
/// Highly robust fallback mechanism to handle symlinks and un-canonicalizable transient paths.
fn get_depth(path: &Path, bases: &[PathBuf], raw_bases: &[PathBuf]) -> Option<usize> {
    let clean = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut min_depth = None;

    for (i, b) in bases.iter().enumerate() {
        // 1. Try canonicalized path against canonicalized base
        if let Ok(stripped) = clean.strip_prefix(b) {
            let d = stripped.components().count();
            min_depth = Some(min_depth.map_or(d, |min| std::cmp::min(min, d)));
        }
        
        // 2. Fallback: Try raw path against canonicalized base
        if let Ok(stripped) = path.strip_prefix(b) {
            let d = stripped.components().count();
            min_depth = Some(min_depth.map_or(d, |min| std::cmp::min(min, d)));
        }
        
        // 3. Fallback: Try raw path against raw base (preserves exact string matches)
        if let Ok(stripped) = path.strip_prefix(&raw_bases[i]) {
            let d = stripped.components().count();
            min_depth = Some(min_depth.map_or(d, |min| std::cmp::min(min, d)));
        }
    }
    
    min_depth
}

/// Recursively scans, watches, and indexes a path while respecting depth limits.
/// O(1) execution for files and known directories to prevent freeze loops.
fn process_path(
    watcher: &mut RecommendedWatcher,
    watched: &mut HashSet<PathBuf>,
    path: &Path,
    max_depth: usize,
    pipeline: &Arc<IngestionPipeline>,
    base_paths: &[PathBuf],
    raw_base_paths: &[PathBuf],
    skip_indexing: bool,
) {
    if !path.exists() || is_hidden(path) {
        pipeline.remove_file(path);
        return;
    }

    let root_depth = match get_depth(path, base_paths, raw_base_paths) {
        Some(d) if d <= max_depth => d,
        _ => return, // Path is not within target boundaries or exceeds max depth
    };

    // Fast-path: Individual files are indexed instantly without directory traversal.
    if path.is_file() {
        if !skip_indexing { pipeline.index_file(path); }
        return;
    }

    if path.is_dir() {
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        
        // Fast-path: If the directory is already watched, do NOT re-traverse it.
        // Any individual files modified inside it will trigger their own isolated events.
        if watched.contains(&key) {
            if !skip_indexing { pipeline.index_file(path); }
            return;
        }

        // If it's a NEW directory (or initial boot), traverse it to place inotify watches
        // on all its subdirectories up to max_depth.
        let remaining_depth = max_depth - root_depth;
        
        for entry in WalkDir::new(path).max_depth(remaining_depth).follow_links(true).into_iter().flatten() {
            let sub_path = entry.path();
            
            if is_hidden(sub_path) { continue; }

            if sub_path.is_dir() {
                let sub_key = sub_path.canonicalize().unwrap_or_else(|_| sub_path.to_path_buf());
                if watched.insert(sub_key) {
                    let _ = watcher.watch(sub_path, RecursiveMode::NonRecursive);
                }
            }
            
            if !skip_indexing { pipeline.index_file(sub_path); }
        }
    }
}

impl IndexTrigger for INotifyTrigger {
    fn name(&self) -> &'static str {
        "Kernel INotify Watcher"
    }

    fn start(&self, target_dirs: Vec<String>, max_depth: usize, pipeline: Arc<IngestionPipeline>) {
        thread::spawn(move || {
            let (tx, rx) = mpsc::channel();
            let (watch_tx, watch_rx) = mpsc::channel::<WatchMessage>();
            
            let mut watcher = RecommendedWatcher::new(
                move |res| { if let Ok(e) = res { let _ = tx.send(e); } },
                Config::default(),
            ).expect("Failed to initialize kernel watcher");

            let mut watched = HashSet::new();
            let mut pending = HashMap::<PathBuf, Instant>::new();
            
            // Retain raw paths for safety fallback if canonicalization drops components
            let raw_base_paths: Vec<PathBuf> = target_dirs.iter()
                .map(|d| PathBuf::from(d))
                .collect();
                
            let base_paths: Vec<PathBuf> = target_dirs.iter()
                .map(|d| Path::new(d).canonicalize().unwrap_or_else(|_| PathBuf::from(d)))
                .collect();

            // 1. Diagnostic Kernel Probe
            let test_dir = std::env::temp_dir().join(format!("lens_for_gnome_inotify_test_{}", std::process::id()));
            let _ = std::fs::create_dir_all(&test_dir);
            if watcher.watch(&test_dir, RecursiveMode::NonRecursive).is_ok() {
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(500));
                    let _ = std::fs::write(test_dir.join("test.txt"), "PING");
                    thread::sleep(Duration::from_millis(500));
                    let _ = std::fs::remove_dir_all(&test_dir);
                });
            }

            // 2. Async Initial Setup (Streaming Watch Placement)
            // We spawn a separate thread to walk the directories because doing it synchronously 
            // blocks the inotify event loop, causing it to miss real-time events while scanning large NAS mounts.
            let target_dirs_clone = target_dirs.clone();
            let base_paths_clone = base_paths.clone();
            let raw_base_paths_clone = raw_base_paths.clone();
            
            thread::spawn(move || {
                for dir in target_dirs_clone {
                    let path = Path::new(&dir);
                    if !path.exists() || is_hidden(path) { 
                        let _ = watch_tx.send(WatchMessage::Done(dir.clone()));
                        continue; 
                    }
                    
                    let root_depth = match get_depth(path, &base_paths_clone, &raw_base_paths_clone) {
                        Some(d) if d <= max_depth => d,
                        _ => {
                            let _ = watch_tx.send(WatchMessage::Done(dir.clone()));
                            continue;
                        }
                    };
                    
                    let remaining_depth = max_depth - root_depth;
                    for entry in WalkDir::new(path).max_depth(remaining_depth).follow_links(true).into_iter().flatten() {
                        let sub_path = entry.path();
                        if !is_hidden(sub_path) && sub_path.is_dir() {
                            let _ = watch_tx.send(WatchMessage::WatchDir(sub_path.to_path_buf()));
                        }
                    }
                    
                    // Send a completion marker to print the log accurately once the directory walk is fully complete
                    let _ = watch_tx.send(WatchMessage::Done(dir.clone()));
                }
            });

            // 3. Event Loop
            loop {
                // 3a. Drain pending watches from the background scanner
                // Limit the batch size per tick to ensure the event loop remains responsive 
                // to live real-time file events even while the background scanner is catching up.
                let mut watch_batch = 0;
                while let Ok(msg) = watch_rx.try_recv() {
                    match msg {
                        WatchMessage::WatchDir(new_dir) => {
                            let sub_key = new_dir.canonicalize().unwrap_or_else(|_| new_dir.clone());
                            if watched.insert(sub_key) {
                                let _ = watcher.watch(&new_dir, RecursiveMode::NonRecursive);
                            }
                            watch_batch += 1;
                        },
                        WatchMessage::Done(dir_str) => {
                            println!("  -> Kernel watching active on: {} (Max Depth: {})", dir_str, max_depth);
                        }
                    }
                    
                    // Yield back to the kernel event drainer if we've processed a large chunk
                    if watch_batch >= 500 {
                        break;
                    }
                }

                // 3b. Poll kernel events
                match rx.recv_timeout(RX_IDLE_TICK) {
                    Ok(Event { kind, paths, .. }) => {
                        // Broadly capture all modification, creation, rename, and deletion events
                        // while explicitly filtering out pure read accesses to prevent indexing loops.
                        if !matches!(kind, EventKind::Access(_)) {
                            for p in paths {
                                if p.to_string_lossy().contains("lens_for_gnome_inotify_test") {
                                    println!("[INotify] Kernel loopback test successful.");
                                    continue;
                                }
                                
                                if !is_hidden(&p) {
                                    pending.insert(p.clone(), Instant::now() + EVENT_DEBOUNCE);
                                }
                            }
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                    _ => {}
                }

                // 4. Drain Debounced Events
                let now = Instant::now();
                let due: Vec<PathBuf> = pending.iter()
                    .filter(|(_, &time)| time <= now)
                    .map(|(p, _)| p.clone())
                    .collect();

                for path in due {
                    pending.remove(&path);
                    
                    if path.exists() {
                        // Real-time events MUST be indexed
                        process_path(&mut watcher, &mut watched, &path, max_depth, &pipeline, &base_paths, &raw_base_paths, false);
                    } else {
                        pipeline.remove_file(&path);
                        let key = path.canonicalize().unwrap_or_else(|_| path.clone());
                        watched.remove(&key);
                    }
                }
            }
        });
    }
}