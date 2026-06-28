// src/engine/llm/strategy_synthesis.rs
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::domain::SearchResult;
use super::{LlmStrategy, LlmCore, extract_relevant_window};

pub struct SynthesisStrategy;

impl LlmStrategy for SynthesisStrategy {
    type Input = (String, String, Vec<SearchResult>);
    type Output = serde_json::Value;

    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let (query, core_concept, mut context_docs) = input;
        
        if is_cancelled.load(Ordering::Relaxed) {
            return serde_json::json!({
                "answer": "Operation cancelled.",
                "reasoning": "Execution context was explicitly terminated.",
                "confidence_score": 0,
                "confidence_justification": "Cancelled",
                "cited_indices": []
            });
        }

        // Limit to 5 sources to prevent LLM context window exhaustion
        context_docs.truncate(5);
        
        let mut context_block = String::new();
        
        // We no longer feed Document IDs to the LLM. 
        // We strip away all metadata and give it pure text so it can focus 100% on answering.
        if context_docs.is_empty() {
            context_block.push_str("No documents available.\n");
        } else {
            for doc in context_docs.iter() {
                let is_shallow = doc.metadata.get("shallow_index").map(|v| v.as_str()) == Some("true");
                if !is_shallow {
                    let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
                    let safe_content = extract_relevant_window(content, &core_concept, 1200);
                    context_block.push_str(&format!("{}\n\n", safe_content.trim()));
                }
            }
        }

        let current_unix_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let cot_bypass = if core.supports_cot { "<think>\n</think>\n" } else { "" };

        // ==========================================
        // PASS 1: SEMANTIC EXTRACTION (LLM)
        // ==========================================
        let prompt_pass_1 = format!(
            "<|im_start|>system\nYou are a precise local data assistant. Answer the user's question based ONLY on the provided DOCUMENTS. General knowledge is FORBIDDEN. Keep your answer concise. Current UNIX Timestamp: {}<|im_end|>\n\
            <|im_start|>user\n\
            DOCUMENTS:\n\
            {}\n\
            \n\
            QUESTION: {}\n\
            \n\
            If the documents do not contain the answer, reply exactly with: I don't know.<|im_end|>\n\
            <|im_start|>assistant\n\
            {}",
            current_unix_ts, context_block.trim(), query, cot_bypass
        );
        
        let response_pass_1 = core.generate_text("SYNTHESIS_PASS_1", &prompt_pass_1, 400, is_cancelled.clone());
        
        let mut clean_answer = response_pass_1.trim().to_string();
        if let Some(end_idx) = clean_answer.find("</think>") {
            clean_answer = clean_answer[end_idx + 8..].trim().to_string();
        }

        let lower_ans = clean_answer.to_lowercase();
        if lower_ans.is_empty() || lower_ans.contains("i don't know") || lower_ans.contains("i do not know") {
            return serde_json::json!({
                "answer": "I don't know. The requested information is not available in the indexed local files.",
                "reasoning": "Irrelevant or insufficient context.", 
                "confidence_score": 0,
                "confidence_justification": "Irrelevant or insufficient context.",
                "cited_indices": []
            });
        }

        // ==========================================
        // PASS 2: DETERMINISTIC ATTRIBUTION (RUST)
        // ==========================================
        // We eliminate the second LLM call entirely. We tokenize the LLM's answer and find 
        // the highest overlapping document deterministically.
        
        let mut raw_cited_indices = std::collections::HashSet::new();
        let clean_lower_ans = lower_ans.replace(|c: char| !c.is_alphanumeric() && !c.is_whitespace(), "");
        
        // Filter out common stop words to ensure we only match on factual keywords
        let ans_words: Vec<&str> = clean_lower_ans
            .split_whitespace()
            .filter(|w| w.len() > 3 && !["this", "that", "they", "them", "their", "what", "when", "where", "which", "there", "these", "those"].contains(w))
            .collect();

        let mut best_score = 0;
        
        for (i, doc) in context_docs.iter().enumerate() {
            let doc_id = i + 1;
            let is_shallow = doc.metadata.get("shallow_index").map(|v| v.as_str()) == Some("true");
            
            if is_shallow { continue; }

            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet).to_lowercase();
            let mut score = 0;

            // 1. Direct substring match (Strongest signal: The LLM output exactly what it read)
            if content.contains(&lower_ans) {
                score += 1000;
            } else {
                // 2. Keyword overlap (Handles slight LLM paraphrasing)
                for word in &ans_words {
                    if content.contains(word) {
                        score += 10;
                    }
                }
            }

            // 3. Concept fallback (Ensures a tie-break maps back to the search query intent)
            let lower_concept = core_concept.to_lowercase();
            let concept_words: Vec<&str> = lower_concept.split_whitespace().filter(|w| w.len() > 3).collect();
            for word in &concept_words {
                 if content.contains(word) {
                     score += 1;
                 }
            }

            // Assign the citation to the document with the highest mathematical overlap
            if score > 0 {
                if score > best_score {
                    best_score = score;
                    raw_cited_indices.clear();
                    raw_cited_indices.insert(doc_id);
                } else if score == best_score {
                    raw_cited_indices.insert(doc_id);
                }
            }
        }

        // HALLUCINATION TRAP: Validate citations against actual document depth
        let mut valid_cited_indices = std::collections::HashSet::new();
        let mut evidence_blocks = Vec::new();

        for &idx in &raw_cited_indices {
            if idx > 0 && idx <= context_docs.len() {
                let doc = &context_docs[idx - 1];
                valid_cited_indices.insert(idx);
                
                let clean_snip = doc.snippet.replace("<b>", "").replace("</b>", "").replace('\n', " ").trim().to_string();
                if !clean_snip.is_empty() {
                    evidence_blocks.push(format!("Source [{}]: \"...{}...\"", idx, clean_snip));
                }
            }
        }

        // Override: If no overlap could be found, the LLM hallucinated external knowledge
        if raw_cited_indices.is_empty() {
            return serde_json::json!({
                "answer": "I don't know. The requested information is not available in the indexed local files.",
                "reasoning": "Hallucination caught: The generated answer did not mathematically correlate to any indexed document.", 
                "confidence_score": 0,
                "confidence_justification": "Answer failed deterministic attribution.",
                "cited_indices": []
            });
        }

        let mut sorted_indices: Vec<usize> = valid_cited_indices.iter().copied().collect();
        sorted_indices.sort_unstable();

        let final_reasoning = if evidence_blocks.is_empty() {
            "Attributed via semantic overlap.".to_string()
        } else {
            format!("Deterministic Evidence:\n{}", evidence_blocks.join("\n\n"))
        };
        
        serde_json::json!({
            "answer": clean_answer,
            "reasoning": final_reasoning,
            "confidence_score": 100,
            "confidence_justification": "Derived from indexed content",
            "cited_indices": sorted_indices
        })
    }
}