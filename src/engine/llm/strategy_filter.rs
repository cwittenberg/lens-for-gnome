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
        let max_batch_chars = 14_000;

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
                // 1-based index to prevent LLM zero-index hallucinations
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

            // High-Speed Boolean Logit Verification Prompt:
            // Eliminates qualitative text generation. Forces the LLM to output exactly one single token symbol
            // per document ('+' or '-'), yielding a massive ~10x speedup across batch pipelines.
            let prompt = format!(
                "<|im_start|>system\nYou are an ultra-fast boolean filter agent. You must output exactly one character per document without formatting or commentary.<|im_end|>\n\
                <|im_start|>user\n\
                Determine if EACH document meets this criteria: \"{}\"\n\
                \n\
                CRITICAL INSTRUCTIONS:\n\
                For EVERY document ID, respond with a single character choice:\n\
                Use '+' if the document satisfies the criteria.\n\
                Use '-' if the document does NOT satisfy the criteria.\n\
                \n\
                Strict Output Format:\n\
                1 | +\n\
                2 | -\n\
                \n\
                DOCUMENTS:\n{}<|im_end|>\n\
                <|im_start|>assistant\n\
                1 | ",
                condition, docs_block
            );

            // Capped token limits since the high-speed format requires minimal token generation space
            let response = core.generate_text("FAST_FILTER_STRATEGY", &prompt, 256, Arc::clone(&is_cancelled));
            
            // Re-prepend our primed start token so line processing logic stays uniform
            let full_response = format!("1 | {}", response.trim());
            
            let mut matched_indices = Vec::new();
            let mut reasoning_map = std::collections::HashMap::new();

            for line in full_response.lines() {
                let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
                
                if parts.len() >= 2 {
                    let id_str = parts[0].replace("DOC_ID:", "").replace("ID:", "").replace(":", "").trim().to_string();
                    
                    if let Ok(id) = id_str.parse::<usize>() {
                        if id > 0 {
                            let doc_idx = id - 1;
                            let decision_symbol = parts[1].trim();
                            
                            let is_match = decision_symbol.starts_with('+');
                            
                            if is_match {
                                matched_indices.push(doc_idx);
                                reasoning_map.insert(doc_idx, format!("Fulfills evaluation condition: '{}'", condition));
                            } else {
                                reasoning_map.insert(doc_idx, format!("Failed evaluation condition: '{}'", condition));
                            }
                        }
                    }
                }
            }

            for (i, doc) in chunk.iter_mut().enumerate() {
                let is_match = matched_indices.contains(&i);
                doc.ai_matched = Some(is_match);
                
                if let Some(thought) = reasoning_map.get(&i) {
                    doc.ai_reasoning = Some(thought.clone());
                } else {
                    doc.ai_reasoning = Some(if is_match { format!("Condition '{}' = True", condition) } else { format!("Condition '{}' = False", condition) });
                }
            }
            
            all_processed.extend(chunk.iter().cloned());
        }

        all_processed
    }
}