// src/plugins/email.rs
use std::sync::Arc;
use crate::domain::{SearchQuery, SearchResult};
use crate::plugins::PluginTool;
use crate::vector::VectorStore;

pub struct EmailPlugin {
    store: Arc<VectorStore>,
}

impl EmailPlugin {
    pub fn new(store: Arc<VectorStore>) -> Self {
        Self { store }
    }
}

impl PluginTool for EmailPlugin {
    fn id(&self) -> &'static str { "plugin:email" }

    fn name(&self) -> &'static str { "Local Mailbox" }
    
    fn can_fast_handle(&self, query: &SearchQuery) -> bool {
        // Fast-pass routes directly when the user intends to query local emails
        query.raw_text.starts_with("mail:") || query.raw_text.starts_with("email:")
    }
    
    fn execute(&self, query: &SearchQuery) -> Vec<SearchResult> {
        let clean_query = query.raw_text
            .replace("mail:", "")
            .replace("email:", "")
            .trim()
            .to_string();

        if clean_query.is_empty() {
            return vec![];
        }

        // 1. Force the engine to strictly search the new native `.eml` ingestions
        let mut filters = query.metadata_filters.clone();
        filters.insert("filetype".to_string(), "eml".to_string());

        // 2. Supplying a zero vector gracefully downgrades the search to a pure BM25/FTS rank score match,
        // which bypasses fuzzy semantic matching when a user wants an exact mailbox keyword.
        let dummy_vector = vec![0.0; 384];

        let results = self.store.search(
            &dummy_vector,
            &clean_query,
            query.min_timestamp,
            query.max_timestamp,
            &filters,
            self.id(),
            query.prioritize_folders
        );

        // We completely removed the `xdg-open` hack here.
        // `ui.js` now handles the routing autonomously based on whether `gmail_url` is present in the database metadata.
        results
    }
}