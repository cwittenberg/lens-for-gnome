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
    pub fn new(store: Arc<VectorStore>) -> Self {
        // Safe instantiation for #[non_exhaustive] struct
        let mut options = InitOptions::default();
        options.model_name = EmbeddingModel::ParaphraseMLMiniLML12V2;

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
        false 
    }
    
    fn execute(&self, query: &SearchQuery) -> Vec<SearchResult> {
        let mut model = self.ai_model.lock().unwrap();
        
        let target_vector = match model.embed(vec![query.raw_text.clone()], None) {
            Ok(mut embs) => embs.pop().unwrap_or_default(),
            Err(_) => return vec![],
        };

        self.store.search(
            &target_vector,
            &query.raw_text,
            query.min_timestamp,
            query.max_timestamp,
            &query.metadata_filters,
            self.id()
        )
    }
}