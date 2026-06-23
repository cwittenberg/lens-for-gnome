// src/triggers/mod.rs
pub mod inotify_watcher;
pub mod gmail_sync;

// Re-export the trigger so it can be cleanly imported from crate::triggers
pub use inotify_watcher::INotifyTrigger;
pub use gmail_sync::GmailSyncDaemon;

use std::sync::Arc;
use crate::ingestion::IngestionPipeline;

/// Strategy Interface for background index triggers (inotify, cron, webhooks, etc.)
pub trait IndexTrigger: Send + Sync {
    fn name(&self) -> &'static str;
    
    /// Binds the trigger to multiple directories and gives it a thread-safe reference 
    /// to the AI ingestion pipeline, while respecting dynamic max_depth limitations.
    fn start(&self, target_dirs: Vec<String>, max_depth: usize, pipeline: Arc<IngestionPipeline>);
}