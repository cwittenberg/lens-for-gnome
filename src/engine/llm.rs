// src/engine/llm.rs
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;

use crate::domain::{SearchQuery, SearchResult};

#[derive(PartialEq)]
pub enum LlmIntent {
    Skip,
    RefineSearch,
    SynthesizeAnswer,
    FilterResults,
}

pub struct LlmEngine {
    backend: LlamaBackend,
    model: LlamaModel,
}

pub struct LlmService {
    engine: Arc<Mutex<LlmEngine>>,
}

impl LlmService {
    pub fn new() -> Self {
        println!("Initializing llama.cpp Backend...");
        
        let backend = LlamaBackend::init().expect("Failed to initialize C++ backend");
        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(&backend, "/tmp/phi3-q4.gguf", &model_params)
            .expect("Failed to load GGUF model. Did you download it to /tmp/phi3-q4.gguf?");

        Self {
            engine: Arc::new(Mutex::new(LlmEngine { backend, model })),
        }
    }

    fn generate_text(&self, prompt: &str, max_tokens: usize) -> String {
        // Safely acquire the lock, recovering if a previous thread panicked
        let engine = match self.engine.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let n_ctx_limit: u32 = 4096;
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(std::num::NonZeroU32::new(n_ctx_limit)); 
        
        let mut ctx = match engine.model.new_context(&engine.backend, ctx_params) {
            Ok(c) => c,
            Err(_) => return String::new(),
        };

        let mut tokens_list = engine.model.str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .unwrap_or_default();

        // Guard against token limit overflows which caused the worker panic
        let safe_max = if max_tokens > n_ctx_limit as usize { (n_ctx_limit / 2) as usize } else { max_tokens };
        let max_prompt_len = (n_ctx_limit as usize).saturating_sub(safe_max).saturating_sub(10);
        
        if tokens_list.len() > max_prompt_len {
            tokens_list.truncate(max_prompt_len);
        }

        if tokens_list.is_empty() { 
            return String::new(); 
        }

        let mut batch = llama_cpp_2::llama_batch::LlamaBatch::new(n_ctx_limit as usize, 1);
        let last_index = tokens_list.len().saturating_sub(1);
        
        for (i, token) in tokens_list.into_iter().enumerate() {
            if batch.add(token, i as i32, &[0], i == last_index).is_err() {
                break;
            }
        }

        if ctx.decode(&mut batch).is_err() {
            return String::new();
        }

        let mut n_cur = batch.n_tokens();
        let mut output = String::new();
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let absolute_max = (prompt.len() + max_tokens).min(n_ctx_limit as usize);

        while (n_cur as usize) <= absolute_max {
            let candidates = ctx.candidates_ith(batch.n_tokens() - 1);
            
            let mut best_token = engine.model.token_eos();
            let mut max_logit = f32::NEG_INFINITY;

            for cand in candidates {
                if cand.logit() > max_logit {
                    max_logit = cand.logit();
                    best_token = cand.id();
                }
            }

            if best_token == engine.model.token_eos() {
                break;
            }

            let token_str = engine.model.token_to_piece(best_token, &mut decoder, false, None)
                .unwrap_or_default();
            
            output.push_str(&token_str);

            batch.clear();
            if batch.add(best_token, n_cur, &[0], true).is_err() {
                break;
            }
            
            if ctx.decode(&mut batch).is_err() {
                break;
            }
            n_cur += 1;
        }

        output
    }

    pub fn determine_intent(query: &str, explicit_synthesis: bool) -> LlmIntent {
        if explicit_synthesis { return LlmIntent::SynthesizeAnswer; }

        let lower = query.to_lowercase();
        
        let filter_triggers = [
            "less than", "greater than", "more than", "under ", "over ", 
            "below ", "above ", "without ", "exactly ", "minder dan", "meer dan"
        ];
        if filter_triggers.iter().any(|&t| lower.contains(t)) {
            return LlmIntent::FilterResults;
        }

        let synthesis_triggers = [
            "what", "how", "why", "who", "when", "summarize", "explain", 
            "wat", "hoe", "waarom", "wie", "wanneer", "vat samen", "leg uit"
        ];
        if synthesis_triggers.iter().any(|&t| lower.starts_with(t) || lower.contains(&format!(" {} ", t))) {
            return LlmIntent::SynthesizeAnswer;
        }

        let time_triggers = [
            "ago", "geleden", "hace", "vor", 
            "days", "dagen", "días", "tage", "day", "dag", "tag",
            "weeks", "weken", "semanas", "wochen", "week", "semana", "woche",
            "months", "maanden", "meses", "monate", "month", "maand", "monat",
            "years", "jaren", "años", "anos", "jahre", "year", "jaar", "año", "ano", "jahr",
            "last", "vorige", "pasado", "letzte"
        ];
        if time_triggers.iter().any(|&t| lower.contains(t)) {
            return LlmIntent::RefineSearch;
        }

        LlmIntent::Skip
    }

    pub fn apply_temporal_heuristics(&self, query: &mut SearchQuery) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let lower_text = query.raw_text.to_lowercase();

        let prompt = format!(
            "<|user|>\nYou are a JSON-only extraction API. The current UNIX timestamp is {}. \
            Analyze the following search query and extract the time constraints. \
            Return ONLY a raw JSON object with NO markdown formatting. \
            Format: {{\"min_ts\": number_or_null, \"max_ts\": number_or_null, \"clean_query\": \"string_without_time_words\"}} \
            Query: \"{}\"<|end|>\n<|assistant|>\n",
            now, lower_text
        );

        let response = self.generate_text(&prompt, 150);

        if let Some(start) = response.find('{') {
            if let Some(end) = response.rfind('}') {
                let json_str = &response[start..=end];
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(min) = parsed["min_ts"].as_u64() { query.min_timestamp = Some(min); }
                    if let Some(max) = parsed["max_ts"].as_u64() { query.max_timestamp = Some(max); }
                    if let Some(clean) = parsed["clean_query"].as_str() { query.raw_text = clean.to_string(); }
                }
            }
        }
    }

    pub fn filter_with_llm(&self, condition: &str, candidates: Vec<SearchResult>) -> Vec<SearchResult> {
        if candidates.is_empty() { return vec![]; }

        let mut docs_block = String::new();
        for (i, doc) in candidates.iter().enumerate() {
            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
            let safe_content: String = content.chars().take(1000).collect(); 
            docs_block.push_str(&format!("[ID: {}]\n{}\n\n", i, safe_content));
        }

        let prompt = format!(
            "<|user|>\nYou are a strict data filtering AI. \
            Evaluate which of the following documents meet this user condition: \"{}\". \
            Carefully extract numeric values, dates, or facts from each document to check the condition. \
            Return ONLY a raw JSON array of the numeric IDs (e.g. [0, 2]) of the documents that strictly match the condition. \
            If none match, return []. Do NOT include markdown formatting or explanations.\n\n\
            DOCUMENTS:\n{}<|end|>\n<|assistant|>\n",
            condition, docs_block
        );

        let response = self.generate_text(&prompt, 150);

        let mut matched_indices: Vec<usize> = Vec::new();
        let mut search_idx = 0;
        
        while let Some(start_offset) = response[search_idx..].find('[') {
            let start = search_idx + start_offset;
            if let Some(end_offset) = response[start..].find(']') {
                let end = start + end_offset;
                let json_str = &response[start..=end];
                if let Ok(parsed) = serde_json::from_str::<Vec<usize>>(json_str) {
                    matched_indices = parsed;
                    break; 
                }
                search_idx = end + 1;
            } else {
                break;
            }
        }

        candidates.into_iter().enumerate()
            .filter(|(i, _)| matched_indices.contains(i))
            .map(|(_, doc)| doc)
            .collect()
    }

    pub fn generate_synthesis(&self, query: &str, context_docs: &[SearchResult]) -> String {
        let mut context_block = String::new();
        for (i, doc) in context_docs.iter().enumerate() {
            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
            let safe_content: String = content.chars().take(1500).collect();
            context_block.push_str(&format!("Document {}:\n{}\n\n", i + 1, safe_content));
        }

        let prompt = format!(
            "<|user|>\nYou are a helpful AI assistant. Answer the user's query using ONLY the provided context documents. \
            If the answer is not contained in the documents, state that you do not have enough information.\n\n\
            CONTEXT:\n{}\n\n\
            QUERY: {}<|end|>\n<|assistant|>\n",
            context_block, query
        );

        self.generate_text(&prompt, 400)
    }
}