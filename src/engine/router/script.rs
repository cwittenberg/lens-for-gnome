use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};
use fancy_regex::Regex;
use crate::domain::SearchResult;

/// Exposes the strictly configured Rhai Engine so the Router can perform 
/// dry-run syntax compilations in the agentic feedback loop before execution.
pub fn build_rhai_engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();
    
    // --- STRICT SANDBOXING AND SECURITY LIMITS ---
    // Rhai inherently does not have access to the OS shell, file system, or network.
    // The primary attack vector from an LLM is a Denial of Service (infinite loops 
    // or massive memory allocation). These hardware limits prevent daemon freezes.
    engine.set_max_operations(5000);        // Prevent infinite loops
    engine.set_max_expr_depths(50, 50);     // Prevent stack overflow from deep recursion
    engine.set_max_array_size(100);         // Cap memory allocation for arrays
    engine.set_max_map_size(100);           // Cap memory allocation for objects
    engine.set_max_string_size(100_000);    // Cap memory for string manipulation
    
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

    engine
}

/// Primary execution hub for the Scripting Engine Strategy.
/// Accepts a pre-compiled, validated AST guaranteed by the Agentic LLM Loop.
pub fn execute_script(
    ast: &rhai::AST,
    mut candidates: Vec<SearchResult>,
    is_cancelled: Arc<AtomicBool>,
) -> Vec<SearchResult> {
    
    let engine = build_rhai_engine();

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

        let result: Result<bool, Box<rhai::EvalAltResult>> = engine.eval_ast_with_scope(&mut scope, ast);
        
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
        let reason_str = "AI".to_string();
        if let Some(prev) = &doc.ai_reasoning {
            doc.ai_reasoning = Some(format!("{} ∧ {}", prev, reason_str));
        } else {
            doc.ai_reasoning = Some(reason_str);
        }
    }

    candidates
}