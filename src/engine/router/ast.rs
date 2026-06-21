// src/engine/router/ast.rs
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use crate::domain::SearchResult;
use crate::engine::llm::LlmService;

/// Converts a LISP-JSON AST array into a human-readable boolean math string
pub fn ast_to_math_string(ast: &serde_json::Value) -> String {
    if let Some(arr) = ast.as_array() {
        if arr.is_empty() { return String::new(); }
        let op = arr[0].as_str().unwrap_or("").to_uppercase();

        match op.as_str() {
            "AND" | "OR" => {
                let mut parts = Vec::new();
                for i in 1..arr.len() {
                    let part = ast_to_math_string(&arr[i]);
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
                    let inner = ast_to_math_string(&arr[1]);
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
pub fn execute_ast(
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
                    current = execute_ast(&arr[i], current, llm, Arc::clone(&is_cancelled));
                    if current.is_empty() { break; }
                }
                return current;
            }
            "OR" => {
                let mut union_map = std::collections::HashMap::new();
                for i in 1..arr.len() {
                    let res = execute_ast(&arr[i], candidates.clone(), llm, Arc::clone(&is_cancelled));
                    for doc in res {
                        union_map.entry(doc.id.clone()).or_insert(doc);
                    }
                }
                return union_map.into_values().collect();
            }
            "NOT" => {
                if arr.len() >= 2 {
                    let to_exclude = execute_ast(&arr[1], candidates.clone(), llm, Arc::clone(&is_cancelled));
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
                        return execute_ast(&serde_json::json!(["SEARCH", concept]), candidates, llm, Arc::clone(&is_cancelled));
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
                        return execute_ast(&serde_json::json!(["NOT", ["SEARCH", concept]]), candidates, llm, Arc::clone(&is_cancelled));
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
                        return execute_ast(&serde_json::json!(["SEARCH", concept]), candidates, llm, Arc::clone(&is_cancelled));
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