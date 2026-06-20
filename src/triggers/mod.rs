pub mod inotify_watcher;

// Re-export the trigger so it can be cleanly imported from crate::triggers
pub use inotify_watcher::INotifyTrigger;

use std::sync::Arc;
use crate::ingestion::IngestionPipeline;

/// Strategy Interface for background index triggers (inotify, cron, webhooks, etc.)
pub trait IndexTrigger: Send + Sync {
    fn name(&self) -> &'static str;
    
    /// Binds the trigger to a directory and gives it a thread-safe reference 
    /// to the AI ingestion pipeline.
    fn start(&self, target_dir: String, pipeline: Arc<IngestionPipeline>);
}