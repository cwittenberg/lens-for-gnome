use std::path::Path;
use std::sync::Arc;
use std::thread;
use notify::{Watcher, RecursiveMode, Event, EventKind};
use notify::event::ModifyKind;
use crate::ingestion::IngestionPipeline;
use super::IndexTrigger;

pub struct INotifyTrigger;

impl IndexTrigger for INotifyTrigger {
    fn name(&self) -> &'static str { "Kernel INotify Watcher" }

    fn start(&self, target_dir: String, pipeline: Arc<IngestionPipeline>) {
        thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            
            // RecommendedWatcher automatically maps to the most efficient OS implementation
            // (inotify on Linux, kqueue on macOS, ReadDirectoryChanges on Windows).
            let mut watcher = notify::RecommendedWatcher::new(tx, notify::Config::default())
                .expect("Failed to initialize kernel file watcher");
            
            watcher.watch(Path::new(&target_dir), RecursiveMode::Recursive)
                .expect("Failed to start watching target directory");

            for res in rx {
                match res {
                    Ok(Event { kind, paths, .. }) => {
                        match kind {
                            EventKind::Create(_) | EventKind::Modify(ModifyKind::Data(_)) => {
                                for path in paths {
                                    if path.is_file() {
                                        pipeline.index_file(&path);
                                    }
                                }
                            },
                            EventKind::Remove(_) => {
                                for path in paths {
                                    pipeline.remove_file(&path);
                                }
                            },
                            EventKind::Modify(ModifyKind::Name(_)) => {
                                // For renames/moves, verify if the current target is an active file
                                for path in paths {
                                    if path.exists() {
                                        if path.is_file() {
                                            pipeline.index_file(&path);
                                        }
                                    } else {
                                        pipeline.remove_file(&path);
                                    }
                                }
                            },
                            _ => {}
                        }
                    },
                    Err(e) => eprintln!("INotify watch error: {:?}", e),
                }
            }
        });
    }
}