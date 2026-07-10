// src/engine/router/mod.rs
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
use crate::engine::RuntimeAdapter;

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn parse_date_to_timestamp(date_str: &str) -> Option<u64> {
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() == 3 {
        if let (Ok(y), Ok(m), Ok(d)) = (parts[0].parse::<i32>(), parts[1].parse::<u32>(), parts[2].parse::<u32>()) {
            let mut days = 0;
            for year in 1970..y {
                days += if is_leap_year(year) { 366 } else { 365 };
            }
            
            let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            for month in 0..(m - 1) as usize {
                if month == 1 && is_leap_year(y) {
                    days += 29;
                } else {
                    days += month_days[month];
                }
            }
            days += (d - 1) as i32;
            
            return Some((days as u64) * 86400);
        }
    }
    None
}

pub struct SystemRouter {
    plugins: Vec<Box<dyn PluginTool>>,
    llm: Arc<LlmService>,
    store: Arc<VectorStore>,
    domain_keywords: HashMap<String, Vec<String>>,
    vision: Arc<VisionEngine>,
    runtime_adapter: Arc<RuntimeAdapter>,
}

impl SystemRouter {
    pub fn new(plugins: Vec<Box<dyn PluginTool>>, store: Arc<VectorStore>, config_dir: &str, runtime_adapter: Arc<RuntimeAdapter>) -> Self {
        let config_path = format!("{}/domains.json", config_dir);
        let domain_keywords = Self::load_config(&config_path);
        let vision = Arc::new(VisionEngine::new(Arc::clone(&runtime_adapter)));
        
        Self { 
            plugins,
            llm: Arc::new(LlmService::new()),
            store,
            domain_keywords,
            vision,
            runtime_adapter,
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
            if ipc::handle_ipc_action(
                json,
                &self.llm,
                &self.vision,
                &self.store,
                &self.runtime_adapter,
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
        
        let enable_ai_filtering = match parsed {
            Ok(ref json) => json["enable_ai_filtering"].as_bool().unwrap_or(true),
            Err(_) => true,
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
            directory_filter: None,
            enable_ai_filtering,
            prioritize_folders,
        };

        if search_query.is_synthesis_request {
            search_query.raw_text = search_query.raw_text[1..].trim().to_string();
        }
        
        let mut current_query = search_query.raw_text.clone();
        let lower_query = current_query.to_lowercase();
        
        // FIX: Ensure correct HOME dir for Snaps and fallback correctly
        let home_dir = std::env::var("SNAP_REAL_HOME").unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
        
        let dir_markers = ["dir:", "path:"];
        let mut found_dir = false;
        
        for marker in dir_markers {
            if let Some(start) = lower_query.find(marker) {
                let val_start = start + marker.len();
                let rest_orig = &current_query[val_start..];
                
                let mut dir_val = String::new();
                let mut chars_to_consume = 0;
                
                if rest_orig.starts_with('"') {
                    if let Some(end_idx) = rest_orig[1..].find('"') {
                        dir_val = rest_orig[1..=end_idx].to_string(); 
                        chars_to_consume = end_idx + 2; 
                    } else {
                        dir_val = rest_orig[1..].to_string();
                        chars_to_consume = rest_orig.len();
                    }
                } else if rest_orig.starts_with('\'') {
                    if let Some(end_idx) = rest_orig[1..].find('\'') {
                        dir_val = rest_orig[1..=end_idx].to_string();
                        chars_to_consume = end_idx + 2;
                    } else {
                        dir_val = rest_orig[1..].to_string();
                        chars_to_consume = rest_orig.len();
                    }
                } else {
                    let next_marker_pos = rest_orig.to_lowercase().find(" ext:")
                        .or_else(|| rest_orig.to_lowercase().find(" type:"))
                        .or_else(|| rest_orig.to_lowercase().find(" after:"))
                        .or_else(|| rest_orig.to_lowercase().find(" since:"))
                        .or_else(|| rest_orig.to_lowercase().find(" before:"))
                        .unwrap_or(rest_orig.len());
                        
                    let raw_unquoted = rest_orig[..next_marker_pos].trim();
                    let mut candidate = raw_unquoted.to_string();
                    let mut found_valid_dir = false;
                    
                    while !candidate.is_empty() {
                        let expanded_cand = candidate.replace("~", &home_dir);
                        let path_obj = std::path::Path::new(&expanded_cand);
                        if path_obj.is_dir() {
                            dir_val = candidate.clone();
                            chars_to_consume = candidate.len();
                            found_valid_dir = true;
                            break;
                        }
                        
                        // Try parent dir fallback for dynamically typed unindexed paths
                        if let Some(last_slash) = candidate.rfind('/') {
                            let parent_len = last_slash + 1;
                            let parent = &candidate[..parent_len];
                            let expanded_parent = parent.replace("~", &home_dir);
                            let parent_obj = std::path::Path::new(&expanded_parent);
                            if parent_obj.is_dir() {
                                dir_val = parent.to_string();
                                chars_to_consume = parent_len;
                                found_valid_dir = true;
                                break;
                            }
                        }
                        if let Some(last_space) = candidate.rfind(' ') {
                            candidate = candidate[..last_space].to_string();
                        } else {
                            break;
                        }
                    }
                    
                    if !found_valid_dir {
                        if let Some(first_space) = raw_unquoted.find(' ') {
                            dir_val = raw_unquoted[..first_space].to_string();
                            chars_to_consume = first_space;
                        } else {
                            dir_val = raw_unquoted.to_string();
                            chars_to_consume = raw_unquoted.len();
                        }
                    }
                }
                
                // FIX: Canonicalize path so symlinks match database IDs correctly!
                let expanded_dir = dir_val.replace("~", &home_dir).trim().to_string();
                let canonical_dir = std::path::Path::new(&expanded_dir).canonicalize().unwrap_or_else(|_| std::path::PathBuf::from(&expanded_dir));
                search_query.directory_filter = Some(canonical_dir.to_string_lossy().to_string());
                
                let before = &current_query[..start];
                let after = &current_query[val_start + chars_to_consume..];
                current_query = format!("{} {}", before, after);
                found_dir = true;
                break;
            }
        }
        
        if !found_dir {
            if let Some(start) = current_query.find(" /").map(|i| i + 1)
                .or_else(|| current_query.find(" ~/").map(|i| i + 1))
                .or_else(|| if current_query.starts_with('/') || current_query.starts_with("~/") { Some(0) } else { None })
            {
                let rest_orig = &current_query[start..];
                
                let next_marker_pos = rest_orig.to_lowercase().find(" ext:")
                    .or_else(|| rest_orig.to_lowercase().find(" type:"))
                    .or_else(|| rest_orig.to_lowercase().find(" after:"))
                    .or_else(|| rest_orig.to_lowercase().find(" since:"))
                    .or_else(|| rest_orig.to_lowercase().find(" before:"))
                    .unwrap_or(rest_orig.len());
                
                let raw_unquoted = rest_orig[..next_marker_pos].trim();
                let mut candidate = raw_unquoted.to_string();
                
                while !candidate.is_empty() {
                    let expanded_cand = candidate.replace("~", &home_dir);
                    let path_obj = std::path::Path::new(&expanded_cand);
                    if path_obj.is_dir() {
                        // FIX: Canonicalize path so symlinks match database IDs correctly!
                        let canonical_dir = path_obj.canonicalize().unwrap_or_else(|_| path_obj.to_path_buf());
                        search_query.directory_filter = Some(canonical_dir.to_string_lossy().trim().to_string());
                        
                        let before = &current_query[..start];
                        let after = &current_query[start + candidate.len()..];
                        current_query = format!("{} {}", before, after);
                        break;
                    }
                    
                    if let Some(last_slash) = candidate.rfind('/') {
                        let parent_len = last_slash + 1;
                        let parent = &candidate[..parent_len];
                        let expanded_parent = parent.replace("~", &home_dir);
                        let parent_obj = std::path::Path::new(&expanded_parent);
                        if parent_obj.is_dir() {
                            // FIX: Canonicalize path so symlinks match database IDs correctly!
                            let canonical_parent = parent_obj.canonicalize().unwrap_or_else(|_| parent_obj.to_path_buf());
                            search_query.directory_filter = Some(canonical_parent.to_string_lossy().trim().to_string());
                            
                            let before = &current_query[..start];
                            let after = &current_query[start + parent_len..];
                            current_query = format!("{} {}", before, after);
                            break;
                        }
                    }
                    if let Some(last_space) = candidate.rfind(' ') {
                        candidate = candidate[..last_space].to_string();
                    } else {
                        break;
                    }
                }
            }
        }
        
        let mut clean_text_parts = Vec::new();
        
        for token in current_query.split_whitespace() {
            let lower_token = token.to_lowercase();
            
            if lower_token.starts_with("ext:") || lower_token.starts_with("type:") {
                let val = token.split(':').nth(1).unwrap_or("");
                search_query.metadata_filters.insert("filetype".to_string(), val.to_lowercase());
            } else if lower_token.starts_with("after:") || lower_token.starts_with("since:") {
                let val = token.split(':').nth(1).unwrap_or("");
                if let Some(ts) = parse_date_to_timestamp(val) {
                    search_query.min_timestamp = Some(ts);
                }
            } else if lower_token.starts_with("before:") {
                let val = token.split(':').nth(1).unwrap_or("");
                if let Some(ts) = parse_date_to_timestamp(val) {
                    search_query.max_timestamp = Some(ts);
                }
            } else {
                clean_text_parts.push(token);
            }
        }
        
        search_query.raw_text = clean_text_parts.join(" ");
        let mut dynamic_fs_results = Vec::new();
        
        // Dynamically invoke a live unindexed directory crawl if the router captures a path marker.
        if let Some(dir) = &search_query.directory_filter {
            if std::path::Path::new(dir).is_dir() {
                let fp_start = Instant::now();
                let browse_results = self.store.browse_directory(dir);
                println!("[Router DEBUG] Directory Browse took: {:.2?} (Found {} results)", fp_start.elapsed(), browse_results.len());

                if search_query.raw_text.is_empty() 
                    && search_query.metadata_filters.is_empty() 
                    && search_query.min_timestamp.is_none() 
                    && search_query.max_timestamp.is_none() 
                {
                    send_chunk(serde_json::json!({
                        "status": "final",
                        "mode": "fast_pass",
                        "results": browse_results
                    }).to_string());
                    return;
                } else {
                    // Extract local memory matches to bridge unindexed depth drops.
                    let q = search_query.raw_text.to_lowercase();
                    dynamic_fs_results = browse_results.into_iter().filter(|r| {
                        let mut matches = true;
                        
                        if !q.is_empty() {
                            matches = r.title.to_lowercase().contains(&q) || r.id.to_lowercase().contains(&q);
                        }
                        
                        if matches {
                            for (k, v) in &search_query.metadata_filters {
                                if let Some(doc_v) = r.metadata.get(k) {
                                    if doc_v.to_lowercase() != v.to_lowercase() {
                                        matches = false;
                                        break;
                                    }
                                } else {
                                    matches = false;
                                    break;
                                }
                            }
                        }
                        
                        if matches {
                            if let Some(min) = search_query.min_timestamp {
                                if r.created_at.unwrap_or(0) < min { matches = false; }
                            }
                        }
                        
                        if matches {
                            if let Some(max) = search_query.max_timestamp {
                                if r.created_at.unwrap_or(0) > max { matches = false; }
                            }
                        }
                        matches
                    }).collect();
                }
            }
        }

        let filetypes = vec!["pdf", "docx", "doc", "txt", "csv", "png", "jpg", "xlsx", "xls", "pptx", "odt", "ods", "odp", "directory"];
        for ft in &filetypes {
            if search_query.raw_text.to_lowercase().contains(ft) {
                search_query.metadata_filters.insert("filetype".to_string(), ft.to_string());
            }
        }

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

        let fp_start = Instant::now();
        let mut fast_results = Vec::new();
        for plugin in &self.plugins {
            if plugin.can_fast_handle(&search_query) {
                fast_results.extend(plugin.execute(&search_query));
            }
        }
        
        // Merge seamlessly with dynamic live crawler results ensuring zero duplicates.
        for fs_res in dynamic_fs_results {
            if !fast_results.iter().any(|r| r.id == fs_res.id) {
                fast_results.push(fs_res);
            }
        }

        println!("[Router DEBUG] Plugins & Vector Search took: {:.2?} (Found {} results)", fp_start.elapsed(), fast_results.len());

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

        let intent_start = Instant::now();
        
        let intent = self.llm.determine_intent(
            &search_query.raw_text, 
            search_query.is_synthesis_request, 
            search_query.enable_ai_filtering, 
            Arc::clone(&is_cancelled)
        );
        
        println!("[Router DEBUG] LLM intent determination took: {:.2?}", intent_start.elapsed());

        let phase_start = Instant::now();
        match intent {
            LlmIntent::Skip => {
                send_chunk(serde_json::json!({"status": "done"}).to_string());
                println!("[Router DEBUG] Skip Intent finished in {:.2?} (Returned 0 LLM results)", phase_start.elapsed());
            },
            
            LlmIntent::FilterScript => {
                send_chunk(serde_json::json!({"status": "filtering", "message": "Reasoning..."}).to_string());
                
                let schema_keys = self.store.get_available_metadata_keys();
                let mut generated_script = self.llm.compile_query_to_script(&search_query.raw_text, schema_keys.clone(), Arc::clone(&is_cancelled));
                
                let engine = script::build_rhai_engine();
                let mut final_ast = None;
                let mut attempt = 1;
                
                while attempt <= 3 {
                    if is_cancelled.load(std::sync::atomic::Ordering::Relaxed) { break; }
                    
                    match engine.compile(&generated_script) {
                        Ok(ast) => {
                            send_chunk(serde_json::json!({"status": "filtering", "message": format!("Validating logic (attempt {})...", attempt)}).to_string());
                            
                            let eval_result = self.llm.evaluate_script_logic(&search_query.raw_text, &generated_script, schema_keys.clone(), Arc::clone(&is_cancelled));
                            
                            if eval_result.starts_with("APPROVE") {
                                println!("[Router DEBUG] Script Logic Approved by Critic on attempt {}", attempt);
                                final_ast = Some(ast);
                                break;
                            } else {
                                let feedback = eval_result.replace("DENY |", "").trim().to_string();
                                println!("[Router DEBUG] Script logic rejected by Critic. Feedback: {}", feedback);
                                send_chunk(serde_json::json!({"status": "filtering", "message": format!("Applying Critic feedback (Attempt {})...", attempt)}).to_string());
                                
                                generated_script = self.llm.fix_script_syntax(
                                    &search_query.raw_text, 
                                    &generated_script, 
                                    &format!("Critic rejected the logic with this feedback: {}", feedback), 
                                    schema_keys.clone(), 
                                    Arc::clone(&is_cancelled)
                                );
                                attempt += 1;
                            }
                        },
                        Err(e) => {
                            send_chunk(serde_json::json!({"status": "filtering", "message": format!("Fixing compilation error (Attempt {})...", attempt)}).to_string());
                            println!("[Router DEBUG] Script Compilation Failed: {}. Requesting fix...", e);
                            
                            generated_script = self.llm.fix_script_syntax(
                                &search_query.raw_text, 
                                &generated_script, 
                                &e.to_string(), 
                                schema_keys.clone(), 
                                Arc::clone(&is_cancelled)
                            );
                            attempt += 1;
                        }
                    }
                }
                
                if let Some(ast) = final_ast {
                    send_chunk(serde_json::json!({
                        "status": "filtering", 
                        "message": format!("Executing:\n{}", generated_script)
                    }).to_string());
                    
                    let mut ast_results = fast_results.clone();
                    let survivors = script::execute_script(&ast, fast_results, Arc::clone(&is_cancelled));
                    
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
                            doc.ai_reasoning = Some("Excluded by AI".to_string());
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
                    
                    println!("[Router DEBUG] Agentic FilterScript Intent finished in {:.2?} (Returned {} results)", phase_start.elapsed(), final_payload.len());
                } else {
                    println!("[Router DEBUG] Agentic Script Loop failed after 3 attempts. Dropping to semantic fast results.");
                    send_chunk(serde_json::json!({
                        "status": "final",
                        "mode": "llm_filtered",
                        "results": partial_payload
                    }).to_string());
                }
            },

            LlmIntent::RefineSearch => {
                send_chunk(serde_json::json!({"status": "processing", "message": "Applying semantic boundaries..."}).to_string());
                
                self.llm.apply_temporal_heuristics(&mut search_query, Arc::clone(&is_cancelled));

                let mut llm_results = Vec::new();
                if let Some(vector_plugin) = self.plugins.iter().find(|p| p.id() == "plugin:vector_db") {
                    llm_results = vector_plugin.execute(&search_query);
                }
                
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

                println!("[Router DEBUG] RefineSearch Intent finished in {:.2?} (Returned {} results)", phase_start.elapsed(), final_payload.len());
            },
            
            LlmIntent::SynthesizeAnswer => {
                send_chunk(serde_json::json!({"status": "synthesizing", "message": "Extracting search concepts..."}).to_string());
                
                let core_concept = self.llm.extract_core_concept(&search_query.raw_text, Arc::clone(&is_cancelled));
                
                send_chunk(serde_json::json!({"status": "synthesizing", "message": "Reading local documents..."}).to_string());
                
                let mut rag_docs = Vec::new();
                if let Some(vector_plugin) = self.plugins.iter().find(|p| p.id() == "plugin:vector_db") {
                    rag_docs = vector_plugin.execute(&search_query);
                }
                
                let answer_json = self.llm.generate_synthesis(&search_query.raw_text, &core_concept, rag_docs.clone(), Arc::clone(&is_cancelled));
                
                let cited_indices: Vec<usize> = answer_json["cited_indices"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as usize)).collect())
                    .unwrap_or_default();
                    
                let mut final_payload = Vec::new();
                
                for idx in cited_indices {
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
                    "results": final_payload
                }).to_string());

                println!("[Router DEBUG] SynthesizeAnswer Intent finished in {:.2?} (Returned {} results)", phase_start.elapsed(), final_payload.len());
            }
        }

        println!("[Router DEBUG] Total request routing took: {:.2?}", req_start.elapsed());
    }
}