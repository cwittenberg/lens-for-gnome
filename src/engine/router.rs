// src/engine/router.rs
use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::SearchQuery;
use crate::plugins::PluginTool;
use super::llm::{LlmService, LlmIntent};

pub struct SystemRouter {
    plugins: Vec<Box<dyn PluginTool>>,
    llm: Arc<LlmService>,
    domain_keywords: HashMap<String, Vec<String>>,
}

impl SystemRouter {
    pub fn new(plugins: Vec<Box<dyn PluginTool>>, config_dir: &str) -> Self {
        let config_path = format!("{}/domains.json", config_dir);
        let domain_keywords = Self::load_config(&config_path);

        Self { 
            plugins,
            llm: Arc::new(LlmService::new()),
            domain_keywords,
        }
    }

    fn load_config(path: &str) -> HashMap<String, Vec<String>> {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(parsed) = serde_json::from_str(&content) {
                return parsed;
            }
        }
        HashMap::new() 
    }

    pub fn handle_request<F>(&self, request_payload: &str, mut send_chunk: F)
    where
        F: FnMut(String),
    {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(request_payload);
        let query_text = match parsed {
            Ok(json) => json["query"].as_str().unwrap_or("").to_string(),
            Err(_) => {
                send_chunk(r#"{"status": "error", "message": "Invalid JSON"}"#.to_string());
                return;
            }
        };

        if query_text.trim().is_empty() {
            send_chunk(r#"{"status": "done", "results": []}"#.to_string());
            return;
        }

        let mut search_query = SearchQuery {
            raw_text: query_text.trim().to_string(),
            is_synthesis_request: query_text.starts_with('?'),
            min_timestamp: None,
            max_timestamp: None,
            metadata_filters: HashMap::new(),
        };

        if search_query.is_synthesis_request {
            search_query.raw_text = search_query.raw_text[1..].trim().to_string();
        }

        let filetypes = vec!["pdf", "docx", "txt", "csv", "png", "jpg", "xlsx"];
        for ft in &filetypes {
            if search_query.raw_text.to_lowercase().contains(ft) {
                search_query.metadata_filters.insert("filetype".to_string(), ft.to_string());
            }
        }

        let lower_q = search_query.raw_text.to_lowercase();
        for (domain, keywords) in &self.domain_keywords {
            if keywords.iter().any(|k| lower_q.contains(k)) {
                search_query.metadata_filters.insert("domain".to_string(), domain.clone());
                break;
            }
        }

        let intent = LlmService::determine_intent(&search_query.raw_text, search_query.is_synthesis_request);

        let mut fast_results = Vec::new();
        let mut handled = false;

        for plugin in &self.plugins {
            if plugin.can_fast_handle(&search_query) {
                fast_results = plugin.execute(&search_query);
                handled = true;
                break;
            }
        }

        if !handled {
            if let Some(vector_plugin) = self.plugins.iter().find(|p| p.id() == "plugin:vector_db") {
                fast_results = vector_plugin.execute(&search_query);
            }
        }

        // We strip the payload context before piping to the socket
        let partial_payload: Vec<_> = fast_results.iter().map(|r| {
            let mut c = r.clone();
            c.full_context = None;
            c
        }).collect();

        send_chunk(serde_json::json!({
            "status": "partial",
            "mode": "fast_pass",
            "results": partial_payload
        }).to_string());

        match intent {
            LlmIntent::Skip => {
                send_chunk(serde_json::json!({"status": "done"}).to_string());
            },
            
            LlmIntent::FilterResults => {
                send_chunk(serde_json::json!({"status": "filtering", "message": "Evaluating documents against condition..."}).to_string());
                
                let filtered = self.llm.filter_with_llm(&search_query.raw_text, fast_results);
                
                let final_payload: Vec<_> = filtered.into_iter().map(|mut r| {
                    r.full_context = None;
                    r
                }).collect();

                send_chunk(serde_json::json!({
                    "status": "final",
                    "mode": "llm_filtered",
                    "results": final_payload
                }).to_string());
            },

            LlmIntent::RefineSearch => {
                self.llm.apply_temporal_heuristics(&mut search_query);
                let mut llm_results = Vec::new();
                if let Some(vector_plugin) = self.plugins.iter().find(|p| p.id() == "plugin:vector_db") {
                    llm_results = vector_plugin.execute(&search_query);
                }
                
                llm_results.retain(|res| !partial_payload.iter().any(|fast_res| fast_res.id == res.id));

                let final_payload: Vec<_> = llm_results.into_iter().map(|mut r| {
                    r.full_context = None;
                    r
                }).collect();

                send_chunk(serde_json::json!({
                    "status": "final",
                    "mode": "llm_enhanced",
                    "results": final_payload
                }).to_string());
            },
            
            LlmIntent::SynthesizeAnswer => {
                send_chunk(serde_json::json!({"status": "synthesizing", "message": "Reading documents..."}).to_string());
                
                let answer = self.llm.generate_synthesis(&search_query.raw_text, &fast_results);
                
                let final_payload: Vec<_> = fast_results.into_iter().map(|mut r| {
                    r.full_context = None;
                    r
                }).collect();

                send_chunk(serde_json::json!({
                    "status": "final",
                    "mode": "rag_synthesis",
                    "synthesis_text": answer.trim(),
                    "results": final_payload
                }).to_string());
            }
        }
    }
}