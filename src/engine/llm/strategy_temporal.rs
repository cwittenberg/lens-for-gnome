// src/engine/llm/strategy_temporal.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};
use super::{LlmStrategy, LlmCore};

pub struct TemporalStrategy;

impl LlmStrategy for TemporalStrategy {
    type Input = String;
    type Output = (Option<u64>, Option<u64>, String);

    fn execute(&self, core: &LlmCore, query: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        let prompt = format!(
            "<|im_start|>system\nYou are a strict data extraction tool. Output ONLY pipe-separated values.<|im_end|>\n\
            <|im_start|>user\n\
            Extract time constraints from the query. Current UNIX timestamp: {}.\n\
            Format: MIN_TS|MAX_TS|CLEAN_QUERY\n\
            Use 0 for missing timestamps. Do not output anything else.\n\
            Example: 0|1690000000|invoices\n\n\
            Query: \"{}\"<|im_end|>\n\
            <|im_start|>assistant\n\
            TEMPORAL_DATA: ",
            now, query
        );

        let response = core.generate_text("TEMPORAL_STRATEGY", &prompt, 50, is_cancelled);
        let clean_response = response.replace("TEMPORAL_DATA:", "");
        let parts: Vec<&str> = clean_response.split('|').collect();
        
        let mut min_ts = None;
        let mut max_ts = None;
        let mut clean_query = String::new();

        if parts.len() >= 3 {
            if let Ok(min) = parts[0].trim().parse::<u64>() { 
                if min > 0 { min_ts = Some(min); } 
            }
            if let Ok(max) = parts[1].trim().parse::<u64>() { 
                if max > 0 { max_ts = Some(max); } 
            }
            clean_query = parts[2..].join("|").trim().to_string();
        }

        (min_ts, max_ts, clean_query)
    }
}