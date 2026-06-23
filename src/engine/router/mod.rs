// src/engine/router/mod.rs
mod ast;
mod script;
mod ipc;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::domain::SearchQuery;
use crate::plugins::PluginTool;
use crate::vector::VectorStore;
use crate::engine::llm::{LlmService, LlmIntent};
use crate::engine::vision::VisionEngine;

pub struct SystemRouter {
    plugins: Vec<Box<dyn PluginTool>>,
    llm: Arc<LlmService>,
    store: Arc<VectorStore>,
    domain_keywords: HashMap<String, Vec<String>>,
    vision: Arc<VisionEngine>,
}

impl SystemRouter {
    pub fn new(plugins: Vec<Box<dyn PluginTool>>, store: Arc<VectorStore>, config_dir: &str) -> Self {
        let config_path = format!("{}/domains.json", config_dir);
        let domain_keywords = Self::load_config(&config_path);

        Self { 
            plugins,
            llm: Arc::new(LlmService::new()),
            store,
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

    pub fn handle_request<F>(&self, request_payload: &str, is_cancelled: Arc<AtomicBool>, mut send_chunk: F)
    where
        F: FnMut(String),
    {
        let req_start = Instant::now();
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(request_payload);
        
        if let Ok(ref json) = parsed {
            // PHASE 0: Intercept Configuration Generic Requests
            if ipc::handle_ipc_action(
                json,
                &self.llm,
                &self.vision,
                &self.store, // Passed the vector store for maintenance IPC commands
                Arc::clone(&is_cancelled),
                req_start,
                &mut send_chunk
            ) {
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

        let filter_strategy = match parsed {
            Ok(ref json) => json["filter_strategy"].as_str().map(|s| s.to_string()),
            Err(_) => None,
        };

        let prioritize_folders = match parsed {
            Ok(ref json) => json["prioritize_folders"].as_bool().unwrap_or(true),
            Err(_) => true,
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
            filter_strategy,
            prioritize_folders,
        };

        if search_query.is_synthesis_request {
            search_query.raw_text = search_query.raw_text[1..].trim().to_string();
        }

        let filetypes = vec!["pdf", "docx", "txt", "csv", "png", "jpg", "xlsx", "directory"];
        for ft in &filetypes {
            if search_query.raw_text.to_lowercase().contains(ft) {
                search_query.metadata_filters.insert("filetype".to_string(), ft.to_string());
            }
        }

        // Alias "folder" queries directly to the "directory" database metadata tag
        if search_query.raw_text.to_lowercase().contains("folder") {
            search_query.metadata_filters.insert("filetype".to_string(), "directory".to_string());
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

        // Run ALL applicable plugins and collect results to enable UI grouping
        for plugin in &self.plugins {
            if plugin.can_fast_handle(&search_query) {
                fast_results.extend(plugin.execute(&search_query));
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
        
        let intent = self.llm.determine_intent(&search_query.raw_text, search_query.is_synthesis_request, search_query.filter_strategy.clone(), Arc::clone(&is_cancelled));
        
        println!("[Router DEBUG] LLM intent determination took: {:.2?}", intent_start.elapsed());

        let phase_start = Instant::now();
        match intent {
            LlmIntent::Skip => {
                send_chunk(serde_json::json!({"status": "done"}).to_string());
                println!("[Router DEBUG] Skip Intent finished in {:.2?}", phase_start.elapsed());
            },
            
            LlmIntent::FilterAst => {
                send_chunk(serde_json::json!({"status": "filtering", "message": "Compiling Logic AST..."}).to_string());
                
                // 1. Fetch available schema keys natively from the Vector Engine
                let schema_keys = self.store.get_available_metadata_keys();
                
                // 2. Translate Natural Language into strict LISP AST via LLM
                let generated_ast = self.llm.compile_query_to_ast(&search_query.raw_text, schema_keys, Arc::clone(&is_cancelled));
                
                // Feedback loop: show the user the mathematical logic derived from their text
                let math_str = ast::ast_to_math_string(&generated_ast);
                let display_str = if math_str.is_empty() { generated_ast.to_string() } else { math_str };
                send_chunk(serde_json::json!({
                    "status": "filtering", 
                    "message": format!("Executing Filter: {}", display_str)
                }).to_string());
                
                // 3. Execute AST natively over the fast-pass candidates
                let mut ast_results = fast_results.clone();
                let survivors = ast::execute_ast(&generated_ast, fast_results, &self.llm, Arc::clone(&is_cancelled));
                
                let mut survivor_map = HashMap::new();
                for s in survivors {
                    survivor_map.insert(s.id.clone(), s);
                }
                
                for doc in &mut ast_results {
                    if let Some(survivor) = survivor_map.get(&doc.id) {
                        doc.ai_matched = Some(true);
                        doc.ai_reasoning = survivor.ai_reasoning.clone();
                    } else {
                        doc.ai_matched = Some(false);
                        doc.ai_reasoning = Some("Excluded by execution graph".to_string());
                    }
                }
                
                // Sort so that AI matches rank highest, followed by false or un-evaluated matches
                ast_results.sort_by(|a, b| {
                    let a_match = a.ai_matched.unwrap_or(false);
                    let b_match = b.ai_matched.unwrap_or(false);
                    b_match.cmp(&a_match) 
                });

                let final_payload: Vec<_> = ast_results.into_iter().map(|mut r| {
                    r.full_context = None;
                    r
                }).collect();

                send_chunk(serde_json::json!({
                    "status": "final",
                    "mode": "llm_filtered",
                    "results": final_payload
                }).to_string());
                println!("[Router DEBUG] FilterAst Intent finished in {:.2?}", phase_start.elapsed());
            },

            LlmIntent::FilterScript => {
                send_chunk(serde_json::json!({"status": "filtering", "message": "Compiling Logic Script..."}).to_string());
                
                let schema_keys = self.store.get_available_metadata_keys();
                let generated_script = self.llm.compile_query_to_script(&search_query.raw_text, schema_keys, Arc::clone(&is_cancelled));
                
                send_chunk(serde_json::json!({
                    "status": "filtering", 
                    "message": format!("Executing Filter Script:\n{}", generated_script)
                }).to_string());
                
                let mut ast_results = fast_results.clone();
                let survivors = script::execute_script(&generated_script, fast_results, &self.llm, Arc::clone(&is_cancelled));
                
                let mut survivor_map = HashMap::new();
                for s in survivors {
                    survivor_map.insert(s.id.clone(), s);
                }
                
                for doc in &mut ast_results {
                    if let Some(survivor) = survivor_map.get(&doc.id) {
                        doc.ai_matched = Some(true);
                        doc.ai_reasoning = survivor.ai_reasoning.clone();
                    } else {
                        doc.ai_matched = Some(false);
                        doc.ai_reasoning = Some("Excluded by execution script".to_string());
                    }
                }
                
                ast_results.sort_by(|a, b| {
                    let a_match = a.ai_matched.unwrap_or(false);
                    let b_match = b.ai_matched.unwrap_or(false);
                    b_match.cmp(&a_match) 
                });

                let final_payload: Vec<_> = ast_results.into_iter().map(|mut r| {
                    r.full_context = None;
                    r
                }).collect();

                send_chunk(serde_json::json!({
                    "status": "final",
                    "mode": "llm_filtered",
                    "results": final_payload
                }).to_string());
                println!("[Router DEBUG] FilterScript Intent finished in {:.2?}", phase_start.elapsed());
            },

            LlmIntent::RefineSearch => {
                send_chunk(serde_json::json!({"status": "processing", "message": "Applying semantic boundaries..."}).to_string());
                
                self.llm.apply_temporal_heuristics(&mut search_query, Arc::clone(&is_cancelled));
                let mut llm_results = Vec::new();
                if let Some(vector_plugin) = self.plugins.iter().find(|p| p.id() == "plugin:vector_db") {
                    llm_results = vector_plugin.execute(&search_query);
                }
                
                // Deduplicate results that were already in the fast pass
                llm_results.retain(|res| !partial_payload.iter().any(|fast_res| fast_res.id == res.id));

                let mut combined_results = fast_results;
                combined_results.extend(llm_results);

                let final_payload: Vec<_> = combined_results.into_iter().map(|mut r| {
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
                send_chunk(serde_json::json!({"status": "synthesizing", "message": "Extracting search concepts..."}).to_string());
                
                let core_concept = self.llm.extract_core_concept(&search_query.raw_text, Arc::clone(&is_cancelled));
                
                send_chunk(serde_json::json!({"status": "synthesizing", "message": "Reading local documents..."}).to_string());
                
                let mut refined_query = search_query.clone();
                // Overwrite the raw text with the clean semantic keywords so vector search isolates the topic
                refined_query.raw_text = core_concept.clone();
                
                let mut rag_docs = Vec::new();
                if let Some(vector_plugin) = self.plugins.iter().find(|p| p.id() == "plugin:vector_db") {
                    rag_docs = vector_plugin.execute(&refined_query);
                }
                
                // Pass the original question to the LLM so it can answer it properly using the clean RAG docs
                let answer_json = self.llm.generate_synthesis(&search_query.raw_text, rag_docs.clone(), Arc::clone(&is_cancelled));
                
                let cited_indices: Vec<usize> = answer_json["cited_indices"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as usize)).collect())
                    .unwrap_or_default();
                    
                let mut final_payload = Vec::new();
                
                for idx in cited_indices {
                    // LLM citations are 1-based (Source [1], Source [2])
                    if idx > 0 && idx <= rag_docs.len() {
                        let mut doc = rag_docs[idx - 1].clone();
                        doc.full_context = None;
                        doc.ai_matched = Some(true);
                        doc.ai_reasoning = Some("Referenced by AI synthesis".to_string());
                        final_payload.push(doc);
                    }
                }

                send_chunk(serde_json::json!({
                    "status": "final",
                    "mode": "rag_synthesis",
                    "synthesis_result": answer_json,
                    // Replaces the bogus fast_results with only the strictly cited files
                    "results": final_payload
                }).to_string());
                println!("[Router DEBUG] SynthesizeAnswer Intent finished in {:.2?}", phase_start.elapsed());
            }
        }

        println!("[Router DEBUG] Total request routing took: {:.2?}", req_start.elapsed());
    }
}