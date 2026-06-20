// src/plugins/email.rs
use crate::domain::{SearchQuery, SearchResult};
use crate::plugins::PluginTool;
use std::collections::HashMap;

pub struct EmailPlugin;

impl PluginTool for EmailPlugin {
    fn id(&self) -> &'static str { "plugin:email" }
    fn name(&self) -> &'static str { "Local Mailbox" }
    
    fn can_fast_handle(&self, query: &SearchQuery) -> bool {
        query.raw_text.starts_with("mail:") || query.raw_text.starts_with("email:")
    }
    
    fn execute(&self, query: &SearchQuery) -> Vec<SearchResult> {
        let clean_query = query.raw_text.replace("mail:", "").replace("email:", "").trim().to_string();
        let mut results = Vec::new();
        
        if clean_query.to_lowercase().contains("invoice") {
            results.push(SearchResult {
                id: "email_hash_892374".to_string(),
                title: "Subject: AWS Invoice for July".to_string(),
                snippet: "From: billing@aws.com - Date: 2026-07-01".to_string(),
                plugin_id: self.id().to_string(),
                score: 0.98,
                filename: None,
                filepath: None,
                metadata: HashMap::new(),
                created_at: None,
                indexed_at: None,
                full_context: None,
            });
        }
        results
    }
}