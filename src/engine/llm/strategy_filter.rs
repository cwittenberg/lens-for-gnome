// src/engine/llm/strategy_filter.rs
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::domain::SearchResult;
use super::{LlmStrategy, LlmCore, extract_relevant_window};

pub struct FastFilterStrategy;

impl LlmStrategy for FastFilterStrategy {
    type Input = (String, Vec<SearchResult>);
    type Output = Vec<SearchResult>;

    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let (condition, candidates) = input;
        if candidates.is_empty() { return vec![]; }

        let mut all_processed = Vec::new();
        let max_batch_chars = 12_000;

        let mut chunks: Vec<Vec<SearchResult>> = Vec::new();
        let mut current_chunk = Vec::new();
        let mut current_chars = 0;

        for doc in candidates.into_iter() {
            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
            let safe_content_len = extract_relevant_window(content, &condition, 800).len();
            let estimated_len = safe_content_len + 100; 

            if !current_chunk.is_empty() && current_chars + estimated_len > max_batch_chars {
                chunks.push(current_chunk);
                current_chunk = Vec::new();
                current_chars = 0;
            }
            current_chunk.push(doc);
            current_chars += estimated_len;
        }
        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }

        for mut chunk in chunks {
            if is_cancelled.load(Ordering::Relaxed) { break; }

            let mut docs_block = String::new();
            for (i, doc) in chunk.iter().enumerate() {
                // 1-based index to prevent LLM zero-index ("0" = "None") hallucinations
                let doc_id = i + 1; 
                let is_shallow = doc.metadata.get("shallow_index").map(|v| v.as_str()) == Some("true");
                
                if is_shallow {
                    docs_block.push_str(&format!(
                        "--- START DOCUMENT ID: {} ---\nFILENAME: {}\nMETADATA: {:?}\n(CONTENT UNINDEXED. EVALUATE BY METADATA)\n--- END DOCUMENT ID: {} ---\n\n", 
                        doc_id, doc.title, doc.metadata, doc_id
                    ));
                } else {
                    let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
                    let safe_content = extract_relevant_window(content, &condition, 800); 
                    docs_block.push_str(&format!("--- START DOCUMENT ID: {} ---\n{}\n--- END DOCUMENT ID: {} ---\n\n", doc_id, safe_content, doc_id));
                }
            }

            // Bulletproof JSON Array Prompt: Forces SLMs into strict numeric output
            let prompt = format!(
                "<|im_start|>system\nYou are a strict JSON API. Output ONLY a valid JSON object. No explanations.<|im_end|>\n\
                <|im_start|>user\n\
                Evaluate which documents satisfy this condition: \"{}\".\n\
                \n\
                CRITICAL INSTRUCTIONS:\n\
                Output a JSON object with a single key \"passed\" containing an array of INTEGER Document IDs that satisfy the condition.\n\
                If no documents pass, output an empty array.\n\
                Example output:\n\
                {{\"passed\": [1, 3]}}\n\
                \n\
                DOCUMENTS:\n{}<|im_end|>\n\
                <|im_start|>assistant\n\
                {{\"passed\": [",
                condition, docs_block
            );

            // Dropped token limit from 150 -> 50 because array generation is hyper-efficient
            let response = core.generate_text("FAST_FILTER_STRATEGY", &prompt, 50, Arc::clone(&is_cancelled));
            let clean_resp = response.trim();
            
            let full_response = if clean_resp.starts_with("{\"passed\":") || clean_resp.starts_with('[') {
                clean_resp.to_string()
            } else {
                format!("{{\"passed\": [{}", clean_resp)
            };

            let mut json_str = full_response.clone();
            let mut parsed_val = serde_json::from_str::<serde_json::Value>(&json_str);
            
            // Iteratively salvage prematurely cut-off JSON arrays
            while parsed_val.is_err() && json_str.len() > 5 {
                json_str.pop();
                let test_str = format!("{}]}}", json_str.trim_end_matches(',').trim_end_matches(']'));
                parsed_val = serde_json::from_str::<serde_json::Value>(&test_str);
            }

            let mut matched_indices = Vec::new();
            if let Ok(json_obj) = parsed_val {
                if let Some(arr) = json_obj.get("passed").and_then(|v| v.as_array()) {
                    for val in arr {
                        // Handle cases where the LLM might output integers or strings
                        if let Some(id_u64) = val.as_u64() {
                            let id = id_u64 as usize;
                            if id > 0 {
                                matched_indices.push(id - 1);
                            }
                        } else if let Some(id_str) = val.as_str() {
                            if let Ok(id) = id_str.parse::<usize>() {
                                if id > 0 {
                                    matched_indices.push(id - 1);
                                }
                            }
                        }
                    }
                }
            }

            for (i, doc) in chunk.iter_mut().enumerate() {
                let is_match = matched_indices.contains(&i);
                doc.ai_matched = Some(is_match);
                doc.ai_reasoning = Some(if is_match { format!("Condition '{}' = True", condition) } else { format!("Condition '{}' = False", condition) });
            }
            
            all_processed.extend(chunk.iter().cloned());
        }

        all_processed
    }
}