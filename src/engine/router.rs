// src/engine/router.rs
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::domain::SearchQuery;
use crate::plugins::PluginTool;
use super::llm::{LlmService, LlmIntent};
use super::vision::VisionEngine;
use crate::engine::HardwareManager;

pub struct SystemRouter {
    plugins: Vec<Box<dyn PluginTool>>,
    llm: Arc<LlmService>,
    domain_keywords: HashMap<String, Vec<String>>,
    vision: Arc<VisionEngine>,
}

impl SystemRouter {
    pub fn new(plugins: Vec<Box<dyn PluginTool>>, config_dir: &str) -> Self {
        let config_path = format!("{}/domains.json", config_dir);
        let domain_keywords = Self::load_config(&config_path);

        Self { 
            plugins,
            llm: Arc::new(LlmService::new()),
            domain_keywords,
            vision: Arc::new(VisionEngine::new()),
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
        let req_start = Instant::now();
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(request_payload);
        
        if let Ok(ref json) = parsed {
            // PHASE 0: Intercept Configuration Generic Requests
            if json["action"].as_str() == Some("get_config") {
                let config_dir = format!("{}/.config/gnome-lens", std::env::var("HOME").unwrap_or_default());
                let config_path = format!("{}/models.json", config_dir);
                let content = std::fs::read_to_string(&config_path).unwrap_or_else(|_| "{}".to_string());
                let parsed_config: serde_json::Value = serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
                
                send_chunk(serde_json::json!({
                    "status": "config_data",
                    "data": parsed_config
                }).to_string());
                return;
            }

            if json["action"].as_str() == Some("get_hardware_status") {
                let hw_status = HardwareManager::detect_hardware();
                send_chunk(serde_json::json!({
                    "status": "hardware_data",
                    "data": hw_status
                }).to_string());
                return;
            }

            if json["action"].as_str() == Some("update_config") {
                if let Some(key) = json["key"].as_str() {
                    let value = &json["value"];
                    
                    // Route to specific managers based on the configuration key
                    if key == "active_model" {
                        if let Some(model_id) = value.as_str() {
                            send_chunk(serde_json::json!({"status": "processing", "message": "Initiating model switch..."}).to_string());
                            
                            match self.llm.switch_model(model_id, &mut send_chunk) {
                                Ok(_) => {
                                    send_chunk(serde_json::json!({
                                        "status": "done",
                                        "message": "Model swapped successfully."
                                    }).to_string());
                                },
                                Err(e) => {
                                    send_chunk(serde_json::json!({
                                        "status": "error",
                                        "message": e
                                    }).to_string());
                                }
                            }
                        }
                    }
                }
                return;
            }

            // PHASE 1: Intercept Vision IPC Requests
            if json["action"].as_str() == Some("extract_snip") {
                if let Some(path) = json["path"].as_str() {
                    send_chunk(serde_json::json!({"status": "processing", "message": "Analyzing image..."}).to_string());
                    let result = self.vision.process_image(path);
                    send_chunk(serde_json::json!({
                        "status": "final",
                        "mode": "vision_extraction",
                        "results": result
                    }).to_string());
                } else {
                    send_chunk(r#"{"status": "error", "message": "Missing path for extraction"}"#.to_string());
                }
                println!("[Router DEBUG] Vision extraction completed in {:.2?}", req_start.elapsed());
                return;
            }
        }

        let query_text = match parsed {
            Ok(ref json) => json["query"].as_str().unwrap_or("").to_string(),
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

        // =====================================================================
        // PHASE 1: EXECUTE FAST PASS FIRST (Instant UI Feedback)
        // =====================================================================
        let fp_start = Instant::now();
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
        println!("[Router DEBUG] Plugins & Vector Search took: {:.2?}", fp_start.elapsed());

        // Strip the payload context before piping to the socket to prevent IPC bloat
        let partial_payload: Vec<_> = fast_results.iter().map(|r| {
            let mut c = r.clone();
            c.full_context = None;
            c
        }).collect();

        // Fire the fast pass results to the GNOME frontend instantly
        send_chunk(serde_json::json!({
            "status": "partial",
            "mode": "fast_pass",
            "results": partial_payload
        }).to_string());

        // =====================================================================
        // PHASE 2: ZERO-SHOT INTENT ROUTING (Background LLM execution)
        // =====================================================================
        let intent_start = Instant::now();
        
        // Gated by the fast lexical pre-filter in llm.rs to guarantee <1ms execution on standard queries
        let intent = self.llm.determine_intent(&search_query.raw_text, search_query.is_synthesis_request);
        
        println!("[Router DEBUG] LLM intent determination took: {:.2?}", intent_start.elapsed());

        let phase_start = Instant::now();
        match intent {
            LlmIntent::Skip => {
                send_chunk(serde_json::json!({"status": "done"}).to_string());
                println!("[Router DEBUG] Skip Intent finished in {:.2?}", phase_start.elapsed());
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
                println!("[Router DEBUG] FilterResults Intent finished in {:.2?}", phase_start.elapsed());
            },

            LlmIntent::RefineSearch => {
                send_chunk(serde_json::json!({"status": "processing", "message": "Applying temporal boundaries..."}).to_string());
                
                self.llm.apply_temporal_heuristics(&mut search_query);
                let mut llm_results = Vec::new();
                if let Some(vector_plugin) = self.plugins.iter().find(|p| p.id() == "plugin:vector_db") {
                    llm_results = vector_plugin.execute(&search_query);
                }
                
                // Deduplicate results that were already in the fast pass
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
                println!("[Router DEBUG] RefineSearch Intent finished in {:.2?}", phase_start.elapsed());
            },
            
            LlmIntent::SynthesizeAnswer => {
                send_chunk(serde_json::json!({"status": "synthesizing", "message": "Reading documents..."}).to_string());
                
                // Passed `.clone()` so it takes an owned Vec without killing the fast_results variable
                let answer = self.llm.generate_synthesis(&search_query.raw_text, fast_results.clone());
                
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
                println!("[Router DEBUG] SynthesizeAnswer Intent finished in {:.2?}", phase_start.elapsed());
            }
        }

        println!("[Router DEBUG] Total request routing took: {:.2?}", req_start.elapsed());
    }
}