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
        
        if context_docs.is_empty() {
            context_block.push_str("No local documents found.\n\n");
        } else {
            for (i, doc) in context_docs.iter().enumerate() {
                let is_shallow = doc.metadata.get("shallow_index").map(|v| v.as_str()) == Some("true");
                
                let doc_date = doc.metadata.get("date").or_else(|| doc.metadata.get("created_at")).unwrap_or(&String::from("Unknown Date")).clone();
                let doc_author = doc.metadata.get("from").or_else(|| doc.metadata.get("author")).unwrap_or(&String::from("Unknown Author")).clone();
                
                if is_shallow {
                    context_block.push_str(&format!(
                        "--- BEGIN SOURCE [{}] ---\nFilename: {}\nPath: {:?}\nDate: {}\nAuthor: {}\nContent:\n[SHALLOW FILE METADATA ONLY. CONTENT UNINDEXED AND UNREADABLE.]\n--- END SOURCE [{}] ---\n\n", 
                        i + 1, doc.title, doc.filepath, doc_date, doc_author, i + 1
                    ));
                } else {
                    let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
                    
                    // Use the core_concept (distilled keywords) instead of the raw conversational query
                    // to anchor the sliding window tightly around the factual data paragraphs.
                    let safe_content = extract_relevant_window(content, &core_concept, 1200);
                    
                    context_block.push_str(&format!(
                        "--- BEGIN SOURCE [{}] ---\nFilename: {}\nDate: {}\nAuthor: {}\nContent:\n{}\n--- END SOURCE [{}] ---\n\n", 
                        i + 1, doc.title, doc_date, doc_author, safe_content, i + 1
                    ));
                }
            }
        }

        let current_unix_ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        // Prompt enhanced with stronger negative constraints regarding shallow files
        let prompt = format!(
            "<|im_start|>system\nYou are a helpful and direct local data assistant. You MUST answer the user's question using ONLY the provided local file CONTEXT. General knowledge is FORBIDDEN.\nCurrent System UNIX Timestamp: {}<|im_end|>\n\
            <|im_start|>user\n\
            CRITICAL INSTRUCTIONS:\n\
            1. Answer the question directly, concisely, and naturally based on the CONTEXT.\n\
            2. DO NOT be overly pedantic or skeptical. If a recipe says '4 egg yolks' and the user asks 'how many eggs', simply answer '4 egg yolks'. Do not over-analyze.\n\
            3. If the context does not contain the answer, or if all relevant sources are marked [SHALLOW FILE METADATA ONLY], output exactly: ANSWER: I don't know.\n\
            4. You MUST end your response with a SOURCES block listing the Source IDs you used.\n\n\
            FORMAT:\n\
            ANSWER: <Your concise, helpful answer>\n\
            SOURCES: <Comma-separated numbers, e.g., 1, 3>\n\n\
            CONTEXT:\n{}\n\n\
            QUERY: {}<|im_end|>\n\
            <|im_start|>assistant\n\
            ANSWER: ",
            current_unix_ts, context_block, query
        );
        
        let response = core.generate_text("SYNTHESIS_STRATEGY", &prompt, 400, is_cancelled);
        
        // Safely remove reasoning blocks before extracting citations
        let mut clean_response = response;
        if let Some(end_idx) = clean_response.find("</think>") {
            clean_response = clean_response[end_idx + 8..].trim().to_string();
        }
        let full_response = format!("ANSWER: {}", clean_response);
        
        let mut sources_str = String::new();
        
        let upper_response = full_response.to_uppercase();
        let answer = if let Some(ans_idx) = upper_response.find("ANSWER:") {
            if let Some(src_idx) = upper_response.find("SOURCES:") {
                sources_str = full_response[src_idx + 8..].trim().to_string();
                full_response[ans_idx + 7..src_idx].trim().to_string()
            } else {
                full_response[ans_idx + 7..].trim().to_string()
            }
        } else {
            full_response.clone()
        };

        let lower_ans = answer.to_lowercase();
        if lower_ans.contains("i don't know") || lower_ans.contains("i do not know") {
            return serde_json::json!({
                "answer": "I don't know. The requested information is not available in the indexed local files.",
                "reasoning": "Irrelevant or insufficient context.", 
                "confidence_score": 0,
                "confidence_justification": "Irrelevant or insufficient context.",
                "cited_indices": []
            });
        }
        
        let mut raw_cited_indices = std::collections::HashSet::new();
        // Map all non-digits to spaces, then split by whitespace to prevent parsing errors.
        let numbers_only: String = sources_str.chars().map(|c| if c.is_ascii_digit() { c } else { ' ' }).collect();
        for part in numbers_only.split_whitespace() {
            if let Ok(num) = part.parse::<usize>() {
                raw_cited_indices.insert(num);
            }
        }
        
        for i in 1..=5 {
            let marker1 = format!("[{}]", i);
            let marker2 = format!("Source [{}]", i);
            if answer.contains(&marker1) || answer.contains(&marker2) {
                raw_cited_indices.insert(i);
            }
        }

        // HALLUCINATION TRAP: Validate citations against actual document depth
        let mut valid_cited_indices = std::collections::HashSet::new();
        let mut evidence_blocks = Vec::new();

        for &idx in &raw_cited_indices {
            if idx > 0 && idx <= context_docs.len() {
                let doc = &context_docs[idx - 1];
                let is_shallow = doc.metadata.get("shallow_index").map(|v| v.as_str()) == Some("true");
                
                if !is_shallow {
                    valid_cited_indices.insert(idx);
                    
                    let clean_snip = doc.snippet.replace("<b>", "").replace("</b>", "").replace('\n', " ").trim().to_string();
                    if !clean_snip.is_empty() {
                        evidence_blocks.push(format!("Source [{}]: \"...{}...\"", idx, clean_snip));
                    }
                }
            }
        }

        // Override: If the model provided citations, but they were ALL invalid (shallow or out of bounds)
        if !raw_cited_indices.is_empty() && valid_cited_indices.is_empty() {
            return serde_json::json!({
                "answer": "I don't know. The requested information is not available in the indexed local files.",
                "reasoning": "Hallucination caught: The model attempted to cite unindexed (shallow) files to support an answer likely derived from general pre-trained knowledge.", 
                "confidence_score": 0,
                "confidence_justification": "All cited sources were shallow/unreadable metadata files.",
                "cited_indices": []
            });
        }

        let mut sorted_indices: Vec<usize> = valid_cited_indices.iter().copied().collect();
        sorted_indices.sort_unstable();

        let final_reasoning = if evidence_blocks.is_empty() {
            "No direct quotes extracted.".to_string()
        } else {
            format!("Deterministic Evidence:\n{}", evidence_blocks.join("\n\n"))
        };
        
        serde_json::json!({
            "answer": answer,
            "reasoning": final_reasoning,
            "confidence_score": 100,
            "confidence_justification": "Derived from valid indexed content.",
            "cited_indices": sorted_indices
        })
    }
}