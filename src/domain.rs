// src/domain.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub plugin_id: String,
    pub score: f32,
    pub filename: Option<String>,
    pub filepath: Option<String>,
    pub metadata: HashMap<String, String>,
    pub created_at: Option<u64>,
    pub indexed_at: Option<u64>,
    pub full_context: Option<String>,
    pub ai_matched: Option<bool>,
    pub ai_reasoning: Option<String>,
}

#[derive(Clone, Default, Debug)]
pub struct SearchQuery {
    pub raw_text: String,
    pub is_synthesis_request: bool,
    pub min_timestamp: Option<u64>,
    pub max_timestamp: Option<u64>,
    pub metadata_filters: HashMap<String, String>,
    pub directory_filter: Option<String>,
    pub enable_ai_filtering: bool,
    pub prioritize_folders: bool,
}