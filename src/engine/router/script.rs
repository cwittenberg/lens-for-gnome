// src/engine/router/script.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};
use fancy_regex::Regex;
use crate::domain::SearchResult;
use crate::engine::llm::LlmService;
use super::ast::execute_ast;

/// Primary execution hub for the Scripting Engine Strategy.
pub fn execute_script(
    script: &str,
    mut candidates: Vec<SearchResult>,
    llm: &Arc<LlmService>,
    is_cancelled: Arc<AtomicBool>,
) -> Vec<SearchResult> {
    let mut engine = rhai::Engine::new();
    
    // --- Standard Library Injections ---
    
    // 1. String parser (Shadows Rhai's built-in that panics on empty strings)
    engine.register_fn("parse_float", |s: rhai::ImmutableString| -> rhai::FLOAT {
        let clean_str: String = s.chars()
            .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect();
            
        if clean_str.is_empty() || clean_str == "." || clean_str == "-" {
            return 0.0;
        }
        clean_str.parse::<rhai::FLOAT>().unwrap_or(0.0)
    });

    // 2. Float passthrough (Prevents errors if LLM passes a raw float to parse_float)
    engine.register_fn("parse_float", |f: rhai::FLOAT| -> rhai::FLOAT {
        f
    });

    // 3. Int to float coercion
    engine.register_fn("parse_float", |i: i64| -> rhai::FLOAT {
        i as rhai::FLOAT
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

    // Extract value using Regex (for numerical extraction)
    // Upgraded: Intelligently returns Capture Group 1 if the LLM targets a specific block
    engine.register_fn("regex_extract", |val: rhai::Dynamic, pattern: rhai::ImmutableString| -> String {
        if val.is_unit() { 
            return String::new(); 
        }
        if let Ok(re) = Regex::new(&pattern) {
            if let Ok(Some(caps)) = re.captures(&val.to_string()) {
                if let Some(m) = caps.get(1).or_else(|| caps.get(0)) {
                    return m.as_str().to_string();
                }
            }
        }
        String::new()
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
            return execute_ast(&serde_json::json!(["SEARCH", script]), candidates, llm, Arc::clone(&is_cancelled));
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