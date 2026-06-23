// src/engine/llm/strategy_synthesis.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use crate::domain::SearchResult;
use super::{LlmStrategy, LlmCore, extract_relevant_window};

pub struct SynthesisStrategy;

impl LlmStrategy for SynthesisStrategy {
    type Input = (String, Vec<SearchResult>);
    type Output = serde_json::Value;

    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let (query, mut context_docs) = input;
        
        context_docs.truncate(10); 
        let mut context_block = String::new();
                 
        if context_docs.is_empty() {
            context_block.push_str("No local documents found.\n\n");
        } else {
            for (i, doc) in context_docs.iter().enumerate() {
                let is_shallow = doc.metadata.get("shallow_index").map(|v| v.as_str()) == Some("true");
                             
                if is_shallow {
                    context_block.push_str(&format!(
                        "Source [{}] (SHALLOW FILE METADATA ONLY):\nFilename: {}\nPath: {:?}\nNote: The content of this file is currently unindexed and unreadable. You may inform the user that this file exists and could be relevant.\n\n", 
                        i + 1, doc.title, doc.filepath
                    ));
                } else {
                    let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
                    let safe_content = extract_relevant_window(content, &query, 1000);
                    context_block.push_str(&format!("Source [{}] ({}):\n{}\n\n", i + 1, doc.title, safe_content));
                }
            }
        }

        let prompt = format!(
            "<|im_start|>system\nYou are a highly strict local data assistant. You MUST answer the user's question using ONLY the provided local file CONTEXT. General knowledge is STRICTLY FORBIDDEN. If the local context does not contain the answer, you MUST respond exactly with 'I don't know' and nothing else.<|im_end|>\n\
            <|im_start|>user\n\
            CRITICAL INSTRUCTIONS:\n\
            1. Format your response EXACTLY like this:\n\
            ANSWER: <final concise answer>\n\
            REASONING: <brief 1 sentence derivation>\n\
            2. You MUST cite sources explicitly inside the 'answer' text block (e.g., 'According to Source [1]...').\n\
            3. If the local sources are irrelevant or empty, your ANSWER MUST BE EXACTLY 'I don't know'.\n\n\
            CONTEXT:\n{}\n\n\
            QUERY: {}<|im_end|>\n\
            <|im_start|>assistant\n\
            ANSWER: ",
            context_block, query
        );
        
        let response = core.generate_text("SYNTHESIS_STRATEGY", &prompt, 400, is_cancelled);
        let full_response = format!("ANSWER: {}", response);
                 
        let mut answer = String::new();
        let mut reasoning = String::new();
        
        let upper_response = full_response.to_uppercase();
        if let Some(ans_idx) = upper_response.find("ANSWER:") {
            if let Some(res_idx) = upper_response.find("REASONING:") {
                if ans_idx < res_idx {
                    answer = full_response[ans_idx + 7..res_idx].trim().to_string();
                    reasoning = full_response[res_idx + 10..].trim().to_string();
                } else {
                    answer = full_response[ans_idx + 7..].trim().to_string();
                }
            } else {
                answer = full_response[ans_idx + 7..].trim().to_string();
            }
        }
        
        if answer.is_empty() {
            answer = full_response;
        }

        // Strict fallback evaluation
        let lower_ans = answer.to_lowercase();
        if lower_ans.contains("i don't know") || lower_ans.contains("i do not know") {
            return serde_json::json!({
                "answer": "I don't know. The requested information is not available in the indexed local files.",
                "reasoning": "No relevant local context found.",
                "confidence_score": 0,
                "confidence_justification": "Missing context",
                "cited_indices": []
            });
        }
        
        // Parse cited sources to generate filtered UI list
        let mut cited_indices = std::collections::HashSet::new();
        for i in 1..=10 {
            let marker1 = format!("[{}]", i);
            let marker2 = format!("Source [{}]", i);
            if answer.contains(&marker1) || answer.contains(&marker2) || reasoning.contains(&marker1) || reasoning.contains(&marker2) {
                cited_indices.insert(i);
            }
        }
        
        serde_json::json!({
            "answer": answer,
            "reasoning": reasoning,
            "confidence_score": 100,
            "confidence_justification": "Derived strictly from local RAG context.",
            "cited_indices": cited_indices.into_iter().collect::<Vec<usize>>()
        })
    }
}