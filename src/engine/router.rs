// src/engine/router.rs
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::domain::{SearchQuery, SearchResult};
use crate::plugins::PluginTool;
use crate::vector::VectorStore;
use super::llm::{LlmService, LlmIntent};
use super::vision::VisionEngine;
use crate::engine::HardwareManager;
use fancy_regex::Regex;

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

    /// Converts a LISP-JSON AST array into a human-readable boolean math string
    fn ast_to_math_string(ast: &serde_json::Value) -> String {
        if let Some(arr) = ast.as_array() {
            if arr.is_empty() { return String::new(); }
            let op = arr[0].as_str().unwrap_or("").to_uppercase();

            match op.as_str() {
                "AND" | "OR" => {
                    let mut parts = Vec::new();
                    for i in 1..arr.len() {
                        let part = Self::ast_to_math_string(&arr[i]);
                        if !part.is_empty() {
                            parts.push(part);
                        }
                    }
                    if parts.is_empty() {
                        String::new()
                    } else if parts.len() == 1 {
                        parts[0].clone()
                    } else {
                        let symbol = if op == "AND" { " ∧ " } else { " ∨ " };
                        format!("({})", parts.join(symbol))
                    }
                }
                "NOT" => {
                    if arr.len() >= 2 {
                        let inner = Self::ast_to_math_string(&arr[1]);
                        format!("¬({})", inner)
                    } else {
                        String::new()
                    }
                }
                "CONTAINS" => {
                    if arr.len() >= 2 {
                        format!("∋ \"{}\"", arr[1].as_str().unwrap_or(""))
                    } else {
                        String::new()
                    }
                }
                "EQ" => {
                    if arr.len() >= 3 {
                        let key = arr[1].as_str().unwrap_or("");
                        let val = if let Some(s) = arr[2].as_str() { format!("\"{}\"", s) } else { arr[2].to_string() };
                        format!("{} = {}", key, val)
                    } else {
                        String::new()
                    }
                }
                "NEQ" => {
                    if arr.len() >= 3 {
                        let key = arr[1].as_str().unwrap_or("");
                        let val = if let Some(s) = arr[2].as_str() { format!("\"{}\"", s) } else { arr[2].to_string() };
                        format!("{} ≠ {}", key, val)
                    } else {
                        String::new()
                    }
                }
                "GT" => {
                    if arr.len() >= 3 {
                        let key = arr[1].as_str().unwrap_or("");
                        let val = arr[2].to_string();
                        format!("{} > {}", key, val)
                    } else {
                        String::new()
                    }
                }
                "LT" => {
                    if arr.len() >= 3 {
                        let key = arr[1].as_str().unwrap_or("");
                        let val = arr[2].to_string();
                        format!("{} < {}", key, val)
                    } else {
                        String::new()
                    }
                }
                "SEARCH" => {
                    if arr.len() >= 2 {
                        format!("≅ \"{}\"", arr[1].as_str().unwrap_or(""))
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    /// Recursively executes the Dynamic LISP-JSON Abstract Syntax Tree against a set of candidates.
    /// Retained as a fallback mechanism if script compilation fails.
    fn execute_ast(
        ast: &serde_json::Value,
        mut candidates: Vec<SearchResult>,
        llm: &Arc<LlmService>,
        is_cancelled: Arc<AtomicBool>,
    ) -> Vec<SearchResult> {
        if is_cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return candidates;
        }

        if let Some(arr) = ast.as_array() {
            if arr.is_empty() { return candidates; }
            let op = arr[0].as_str().unwrap_or("").to_uppercase();

            match op.as_str() {
                "AND" => {
                    let mut current = candidates;
                    for i in 1..arr.len() {
                        current = Self::execute_ast(&arr[i], current, llm, Arc::clone(&is_cancelled));
                        if current.is_empty() { break; }
                    }
                    return current;
                }
                "OR" => {
                    let mut union_map = std::collections::HashMap::new();
                    for i in 1..arr.len() {
                        let res = Self::execute_ast(&arr[i], candidates.clone(), llm, Arc::clone(&is_cancelled));
                        for doc in res {
                            union_map.entry(doc.id.clone()).or_insert(doc);
                        }
                    }
                    return union_map.into_values().collect();
                }
                "NOT" => {
                    if arr.len() >= 2 {
                        let to_exclude = Self::execute_ast(&arr[1], candidates.clone(), llm, Arc::clone(&is_cancelled));
                        let exclude_ids: std::collections::HashSet<_> = to_exclude.into_iter().map(|d| d.id).collect();
                        
                        candidates.retain(|doc| !exclude_ids.contains(&doc.id));
                        
                        let reason_str = "Survived exclusionary filter".to_string();
                        for doc in &mut candidates {
                            doc.ai_matched = Some(true);
                            if let Some(prev) = &doc.ai_reasoning {
                                doc.ai_reasoning = Some(format!("{} ∧ {}", prev, reason_str));
                            } else {
                                doc.ai_reasoning = Some(reason_str.clone());
                            }
                        }
                        return candidates;
                    }
                }
                "CONTAINS" => {
                    if arr.len() >= 2 {
                        let substring = arr[1].as_str().unwrap_or("").to_lowercase();
                        candidates.retain(|doc| {
                            let text = doc.full_context.as_deref().unwrap_or(&doc.snippet).to_lowercase();
                            let title = doc.title.to_lowercase();
                            text.contains(&substring) || title.contains(&substring)
                        });
                        
                        let reason_str = format!("Contains '{}'", substring);
                        for doc in &mut candidates {
                            doc.ai_matched = Some(true);
                            if let Some(prev) = &doc.ai_reasoning {
                                doc.ai_reasoning = Some(format!("{} ∧ {}", prev, reason_str));
                            } else {
                                doc.ai_reasoning = Some(reason_str.clone());
                            }
                        }
                        return candidates;
                    }
                }
                "EQ" => {
                    if arr.len() >= 3 {
                        let key = arr[1].as_str().unwrap_or("");
                        let val_str = if let Some(s) = arr[2].as_str() {
                            s.to_lowercase()
                        } else if let Some(n) = arr[2].as_f64() {
                            n.to_string()
                        } else {
                            String::new()
                        };
                        
                        let key_exists = candidates.iter().any(|doc| doc.metadata.contains_key(key));
                        if !key_exists {
                            let concept = format!("{} is {}", key, val_str);
                            return Self::execute_ast(&serde_json::json!(["SEARCH", concept]), candidates, llm, Arc::clone(&is_cancelled));
                        }

                        candidates.retain(|doc| {
                            if let Some(meta_val) = doc.metadata.get(key) {
                                meta_val.to_lowercase() == val_str
                            } else {
                                false
                            }
                        });
                        
                        let reason_str = format!("{} == {}", key, val_str);
                        for doc in &mut candidates {
                            doc.ai_matched = Some(true);
                            if let Some(prev) = &doc.ai_reasoning {
                                doc.ai_reasoning = Some(format!("{} ∧ {}", prev, reason_str));
                            } else {
                                doc.ai_reasoning = Some(reason_str.clone());
                            }
                        }
                        return candidates;
                    }
                }
                "NEQ" => {
                    if arr.len() >= 3 {
                        let key = arr[1].as_str().unwrap_or("");
                        let val_str = if let Some(s) = arr[2].as_str() {
                            s.to_lowercase()
                        } else if let Some(n) = arr[2].as_f64() {
                            n.to_string()
                        } else {
                            String::new()
                        };
                        
                        let key_exists = candidates.iter().any(|doc| doc.metadata.contains_key(key));
                        if !key_exists {
                            let concept = format!("{} is not {}", key, val_str);
                            return Self::execute_ast(&serde_json::json!(["NOT", ["SEARCH", concept]]), candidates, llm, Arc::clone(&is_cancelled));
                        }

                        candidates.retain(|doc| {
                            if let Some(meta_val) = doc.metadata.get(key) {
                                meta_val.to_lowercase() != val_str
                            } else {
                                true
                            }
                        });
                        
                        let reason_str = format!("{} != {}", key, val_str);
                        for doc in &mut candidates {
                            doc.ai_matched = Some(true);
                            if let Some(prev) = &doc.ai_reasoning {
                                doc.ai_reasoning = Some(format!("{} ∧ {}", prev, reason_str));
                            } else {
                                doc.ai_reasoning = Some(reason_str.clone());
                            }
                        }
                        return candidates;
                    }
                }
                "GT" | "LT" => {
                    if arr.len() >= 3 {
                        let key = arr[1].as_str().unwrap_or("");
                        let target_val = arr[2].as_f64().unwrap_or(0.0);
                        
                        let key_exists = candidates.iter().any(|doc| doc.metadata.contains_key(key));
                        if !key_exists {
                            let op_str = if op == "GT" { "greater than" } else { "less than" };
                            let concept = format!("{} is {} {}", key, op_str, target_val);
                            return Self::execute_ast(&serde_json::json!(["SEARCH", concept]), candidates, llm, Arc::clone(&is_cancelled));
                        }
                        
                        candidates.retain(|doc| {
                            if let Some(meta_val) = doc.metadata.get(key) {
                                if let Ok(doc_val) = meta_val.parse::<f64>() {
                                    if op == "GT" { doc_val > target_val } else { doc_val < target_val }
                                } else { false }
                            } else { false }
                        });
                        
                        let reason_str = format!("{} {} {}", key, if op == "GT" { ">" } else { "<" }, target_val);
                        for doc in &mut candidates {
                            doc.ai_matched = Some(true);
                            if let Some(prev) = &doc.ai_reasoning {
                                doc.ai_reasoning = Some(format!("{} ∧ {}", prev, reason_str));
                            } else {
                                doc.ai_reasoning = Some(reason_str.clone());
                            }
                        }
                        return candidates;
                    }
                }
                "SEARCH" => {
                    if arr.len() >= 2 {
                        let concept = arr[1].as_str().unwrap_or("");
                        
                        let mut previous_reasons = HashMap::new();
                        for doc in &candidates {
                            if let Some(r) = &doc.ai_reasoning {
                                previous_reasons.insert(doc.id.clone(), r.clone());
                            }
                        }
                        
                        let filtered = llm.filter_with_llm(concept, candidates, Arc::clone(&is_cancelled));
                        let mut passing = Vec::new();
                        for mut doc in filtered {
                            if doc.ai_matched.unwrap_or(false) {
                                if let Some(prev) = previous_reasons.get(&doc.id) {
                                    let new_reason = doc.ai_reasoning.clone().unwrap_or_default();
                                    doc.ai_reasoning = Some(format!("{} ∧ {}", prev, new_reason));
                                }
                                passing.push(doc);
                            }
                        }
                        return passing;
                    }
                }
                _ => {}
            }
        }
        
        candidates
    }

    /// Primary execution hub for the Scripting Engine Strategy.
    fn execute_script(
        script: &str,
        mut candidates: Vec<SearchResult>,
        llm: &Arc<LlmService>,
        is_cancelled: Arc<AtomicBool>,
    ) -> Vec<SearchResult> {
        let mut engine = rhai::Engine::new();
        
        // --- Standard Library Injections ---
        
        // Type coercion safety
        engine.register_fn("parse_float", |s: rhai::Dynamic| -> rhai::FLOAT {
            if s.is_unit() { 
                return 0.0; 
            }
            if let Some(float_val) = s.clone().try_cast::<rhai::FLOAT>() {
                return float_val;
            }
            if let Some(int_val) = s.clone().try_cast::<i64>() {
                return int_val as rhai::FLOAT;
            }
            s.to_string().parse::<rhai::FLOAT>().unwrap_or(0.0)
        });

        // Basic string tools
        engine.register_fn("contains_ignore_case", |val: rhai::Dynamic, search: rhai::ImmutableString| -> bool {
            if val.is_unit() { 
                return false; 
            }
            val.to_string().to_lowercase().contains(&search.to_lowercase())
        });

        // Fast native Regex evaluation (Powered by fancy-regex)
        engine.register_fn("regex_match", |val: rhai::Dynamic, pattern: rhai::ImmutableString| -> bool {
            if val.is_unit() { 
                return false; 
            }
            if let Ok(re) = Regex::new(&pattern) {
                re.is_match(&val.to_string()).unwrap_or(false)
            } else {
                false
            }
        });

        // Substring array aggregator (simulates a SQL `IN` array)
        engine.register_fn("in_list", |val: rhai::Dynamic, comma_list: rhai::ImmutableString| -> bool {
            if val.is_unit() { 
                return false; 
            }
            let target = val.to_string().to_lowercase();
            comma_list.split(',')
                .map(|s| s.trim().to_lowercase())
                .any(|item| target == item || target.contains(&item) || item.contains(&target))
        });

        // Context-aware temporal math
        engine.register_fn("days_ago", |days: rhai::FLOAT| -> rhai::FLOAT {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as f64;
            now - (days * 86400.0)
        });

        let ast = match engine.compile(script) {
            Ok(ast) => ast,
            Err(e) => {
                println!("[Router DEBUG] Rhai script compilation failed: {}\nScript:\n{}", e, script);
                // Fallback to the AST Search heuristic if the script fails compilation
                return Self::execute_ast(&serde_json::json!(["SEARCH", script]), candidates, llm, Arc::clone(&is_cancelled));
            }
        };

        candidates.retain(|doc| {
            if is_cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return false;
            }

            let mut scope = rhai::Scope::new();
            
            let mut meta_map = rhai::Map::new();
            for (k, v) in &doc.metadata {
                meta_map.insert(k.clone().into(), rhai::Dynamic::from(v.clone()));
            }
            scope.push("metadata", meta_map);
            
            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
            scope.push("text", content.to_string());
            scope.push("title", doc.title.clone());

            let result: Result<bool, Box<rhai::EvalAltResult>> = engine.eval_ast_with_scope(&mut scope, &ast);
            
            match result {
                Ok(true) => true,
                Ok(false) => false,
                Err(e) => {
                    println!("[Router DEBUG] Rhai script runtime error on doc {}: {}", doc.id, e);
                    false
                }
            }
        });
        
        for doc in &mut candidates {
            doc.ai_matched = Some(true);
            let reason_str = "Matched via LLM System Script".to_string();
            if let Some(prev) = &doc.ai_reasoning {
                doc.ai_reasoning = Some(format!("{} ∧ {}", prev, reason_str));
            } else {
                doc.ai_reasoning = Some(reason_str);
            }
        }

        candidates
    }

    pub fn handle_request<F>(&self, request_payload: &str, is_cancelled: Arc<AtomicBool>, mut send_chunk: F)
    where
        F: FnMut(String),
    {
        let req_start = Instant::now();
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(request_payload);
        
        if let Ok(ref json) = parsed {
            // PHASE 0: Intercept Configuration Generic Requests
            if json["action"].as_str() == Some("get_config") {
                let parsed_config = crate::engine::model_manager::ModelManager::get_full_config();
                
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

            if json["action"].as_str() == Some("delete_model") {
                if let Some(model_id) = json["model_id"].as_str() {
                    match crate::engine::model_manager::ModelManager::delete_model(model_id) {
                        Ok(_) => {
                            send_chunk(serde_json::json!({
                                "status": "done",
                                "message": "Model deleted successfully."
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
                return;
            }

            if json["action"].as_str() == Some("update_config") {
                if let Some(key) = json["key"].as_str() {
                    let value = &json["value"];
                    
                    if key == "active_model" {
                        if let Some(model_id) = value.as_str() {
                            send_chunk(serde_json::json!({"status": "processing", "message": "Initiating model switch..."}).to_string());
                            
                            match self.llm.switch_model(model_id, &mut send_chunk, Arc::clone(&is_cancelled)) {
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
            
            // ACTION: App Launcher
            if json["action"].as_str() == Some("launch_app") {
                if let Some(exec) = json["exec"].as_str() {
                    let clean_exec = exec.replace("%u", "")
                        .replace("%U", "")
                        .replace("%f", "")
                        .replace("%F", "")
                        .replace("%c", "");

                    let mut parts = clean_exec.split_whitespace();
                    if let Some(cmd) = parts.next() {
                        let args: Vec<&str> = parts.collect();
                        
                        let spawn_res = std::process::Command::new(cmd)
                            .args(args)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn();

                        match spawn_res {
                            Ok(_) => {
                                send_chunk(serde_json::json!({
                                    "status": "done",
                                    "message": format!("Launched {}", cmd)
                                }).to_string());
                            },
                            Err(e) => {
                                send_chunk(serde_json::json!({
                                    "status": "error",
                                    "message": format!("Failed to launch: {}", e)
                                }).to_string());
                            }
                        }
                    }
                }
                return;
            }

            // ACTION: Universal File Launcher (mlocate integration)
            if json["action"].as_str() == Some("open_file") {
                if let Some(path) = json["path"].as_str() {
                    let spawn_res = std::process::Command::new("xdg-open")
                        .arg(path)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();

                    match spawn_res {
                        Ok(_) => {
                            send_chunk(serde_json::json!({
                                "status": "done",
                                "message": "File opened via native OS handler."
                            }).to_string());
                        },
                        Err(e) => {
                            send_chunk(serde_json::json!({
                                "status": "error",
                                "message": format!("Failed to open file: {}", e)
                            }).to_string());
                        }
                    }
                }
                return;
            }

            // ACTION: Open Folder Location
            if json["action"].as_str() == Some("open_folder") {
                if let Some(path_str) = json["path"].as_str() {
                    let path = std::path::Path::new(path_str);
                    if let Some(parent) = path.parent() {
                        let spawn_res = std::process::Command::new("xdg-open")
                            .arg(parent)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn();

                        match spawn_res {
                            Ok(_) => {
                                send_chunk(serde_json::json!({
                                    "status": "done",
                                    "message": "Folder location opened via native OS handler."
                                }).to_string());
                            },
                            Err(e) => {
                                send_chunk(serde_json::json!({
                                    "status": "error",
                                    "message": format!("Failed to open folder: {}", e)
                                }).to_string());
                            }
                        }
                    } else {
                        send_chunk(serde_json::json!({
                            "status": "error",
                            "message": "Could not determine parent directory."
                        }).to_string());
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

        let filter_strategy = match parsed {
            Ok(ref json) => json["filter_strategy"].as_str().map(|s| s.to_string()),
            Err(_) => None,
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
                let ast = self.llm.compile_query_to_ast(&search_query.raw_text, schema_keys, Arc::clone(&is_cancelled));
                
                // Feedback loop: show the user the mathematical logic derived from their text
                let math_str = Self::ast_to_math_string(&ast);
                let display_str = if math_str.is_empty() { ast.to_string() } else { math_str };
                send_chunk(serde_json::json!({
                    "status": "filtering", 
                    "message": format!("Executing Filter: {}", display_str)
                }).to_string());
                
                // 3. Execute AST natively over the fast-pass candidates
                let mut ast_results = fast_results.clone();
                let survivors = Self::execute_ast(&ast, fast_results, &self.llm, Arc::clone(&is_cancelled));
                
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
                let script = self.llm.compile_query_to_script(&search_query.raw_text, schema_keys, Arc::clone(&is_cancelled));
                
                send_chunk(serde_json::json!({
                    "status": "filtering", 
                    "message": format!("Executing Filter Script:\n{}", script)
                }).to_string());
                
                let mut ast_results = fast_results.clone();
                let survivors = Self::execute_script(&script, fast_results, &self.llm, Arc::clone(&is_cancelled));
                
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
                send_chunk(serde_json::json!({"status": "synthesizing", "message": "Reading documents..."}).to_string());
                
                let answer_json = self.llm.generate_synthesis(&search_query.raw_text, fast_results.clone(), Arc::clone(&is_cancelled));
                
                let final_payload: Vec<_> = fast_results.into_iter().map(|mut r| {
                    r.full_context = None;
                    r
                }).collect();

                send_chunk(serde_json::json!({
                    "status": "final",
                    "mode": "rag_synthesis",
                    "synthesis_result": answer_json,
                    "results": final_payload
                }).to_string());
                println!("[Router DEBUG] SynthesizeAnswer Intent finished in {:.2?}", phase_start.elapsed());
            }
        }

        println!("[Router DEBUG] Total request routing took: {:.2?}", req_start.elapsed());
    }
}