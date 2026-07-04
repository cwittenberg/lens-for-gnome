// src/plugins/vector_search.rs
use std::sync::{Arc, Mutex};
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use crate::domain::{SearchQuery, SearchResult};
use crate::plugins::PluginTool;
use crate::vector::VectorStore;

pub struct VectorSearchPlugin {
    store: Arc<VectorStore>,
    ai_model: Mutex<TextEmbedding>,
}

impl VectorSearchPlugin {
    pub fn new(store: Arc<VectorStore>, data_dir: &str) -> Self {
        let mut options = InitOptions::default();
        options.model_name = EmbeddingModel::ParaphraseMLMiniLML12V2;
        // Explicitly map the download path to the writable shared data directory
        options.cache_dir = std::path::PathBuf::from(data_dir).join("fastembed_cache");

        let ai_model = TextEmbedding::try_new(options)
            .expect("Failed to init Multi-lingual AI for query embedding");

        Self { 
            store, 
            ai_model: Mutex::new(ai_model) 
        }
    }
}

impl PluginTool for VectorSearchPlugin {
    fn id(&self) -> &'static str { "plugin:vector_db" }
    fn name(&self) -> &'static str { "Semantic File Search" }
    
    fn can_fast_handle(&self, _query: &SearchQuery) -> bool {
        true 
    }
    
    fn execute(&self, query: &SearchQuery) -> Vec<SearchResult> {
        // FAST PATH: If the query is less than 3 words, bypass AI semantic embedding 
        // to grant instantaneous (<10ms) Spotlight-like performance for normal keyword searches.
        let word_count = query.raw_text.split_whitespace().count();
        let target_vector = if word_count >= 3 {
            let mut model = self.ai_model.lock().unwrap();
            match model.embed(vec![query.raw_text.clone()], None) {
                Ok(mut embs) => embs.pop().unwrap_or_else(|| vec![0.0; 384]),
                Err(_) => vec![0.0; 384],
            }
        } else {
            vec![0.0; 384] 
        }; 

        self.store.search(
            &target_vector,
            &query.raw_text,
            query.min_timestamp,
            query.max_timestamp,
            &query.metadata_filters,
            query.directory_filter.as_ref(),
            self.id(),
            // Folders are handled by Top Hits in JS now, we just pass the vector payload
            false 
        )
    }
}