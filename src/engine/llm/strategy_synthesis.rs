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

        if context_docs.is_empty() {
            return serde_json::json!({
                "answer": "No documents found.",
                "reasoning": "Vector search returned empty.",
                "confidence_score": 0,
                "confidence_justification": "Missing context"
            });
        }

        context_docs.truncate(4);
        let mut context_block = String::new();
        
        for (i, doc) in context_docs.iter().enumerate() {
            let is_shallow = doc.metadata.get("shallow_index").map(|v| v.as_str()) == Some("true");
            
            if is_shallow {
                context_block.push_str(&format!(
                    "Source [{}] (SHALLOW FILE METADATA ONLY):\nFilename: {}\nPath: {:?}\nNote: The content of this file is currently unindexed and unreadable. You may inform the user that this file exists and could be relevant.\n\n", 
                    i + 1, doc.title, doc.filepath
                ));
            } else {
                let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
                let safe_content = extract_relevant_window(content, &query, 1500);
                context_block.push_str(&format!("Source [{}] ({}):\n{}\n\n", i + 1, doc.title, safe_content));
            }
        }

        let prompt = format!(
            "<|im_start|>system\nYou are an analytical AI. Answer using ONLY the provided context.<|im_end|>\n\
            <|im_start|>user\n\
            CRITICAL INSTRUCTIONS:\n\
            1. Format your response EXACTLY like this, with nothing else:\n\
            ANSWER: <final concise answer citing sources>\n\
            REASONING: <brief 1 sentence derivation>\n\
            2. Cite sources in the 'answer' (e.g., 'According to doc1...').\n\
            3. If a source is a 'SHALLOW FILE', you cannot see its content. Suggest opening it.\n\n\
            CONTEXT:\n{}\n\n\
            QUERY: {}<|im_end|>\n\
            <|im_start|>assistant\n\
            ANSWER: ",
            context_block, query
        );

        let response = core.generate_text("SYNTHESIS_STRATEGY", &prompt, 300, is_cancelled);
        let full_response = format!("ANSWER: {}", response);
        
        let mut answer = String::new();
        let mut reasoning = String::new();

        if let Some(ans_idx) = full_response.find("ANSWER:") {
            if let Some(res_idx) = full_response.find("REASONING:") {
                if ans_idx < res_idx {
                    answer = full_response[ans_idx + 7..res_idx].trim().to_string();
                    reasoning = full_response[res_idx + 10..].trim().to_string();
                }
            } else {
                answer = full_response[ans_idx + 7..].trim().to_string();
            }
        }

        if answer.is_empty() { 
            answer = full_response; 
        }

        serde_json::json!({
            "answer": answer,
            "reasoning": reasoning,
            "confidence_score": 100,
            "confidence_justification": "Derived natively"
        })
    }
}