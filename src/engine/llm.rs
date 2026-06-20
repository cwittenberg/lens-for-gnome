// src/engine/llm.rs
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use std::io::Write;
use std::env;

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
    debug_mode: bool,
}

impl LlmService {
    pub fn new() -> Self {
        println!("Initializing llama.cpp Backend...");
        
        let backend = LlamaBackend::init().expect("Failed to initialize C++ backend");
        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(&backend, "/tmp/phi3-q4.gguf", &model_params)
            .expect("Failed to load GGUF model. Did you download it to /tmp/phi3-q4.gguf?");

        let debug_mode = env::var("DEBUG_LLM_PROMPT").unwrap_or_else(|_| "0".to_string()) == "1";
        if debug_mode {
            println!("[DEBUG] LLM prompt debugging is ENABLED. Raw OCR text and prompts will be printed to stdout.");
        }

        Self {
            engine: Arc::new(Mutex::new(LlmEngine { backend, model })),
            debug_mode,
        }
    }

    fn generate_text(&self, prompt: &str, max_tokens: usize) -> String {
        if self.debug_mode {
            println!("\n=======================================================================================");
            println!("[DEBUG] RAW TEXT/PROMPT FED TO THE LLM:");
            println!("---------------------------------------------------------------------------------------");
            println!("{}", prompt);
            println!("=======================================================================================\n");
        }

        let engine = match self.engine.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let n_ctx_limit: u32 = 4096;
        let n_batch_limit: u32 = 512;
        
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(std::num::NonZeroU32::new(n_ctx_limit))
            .with_n_batch(n_batch_limit);
            
        let mut ctx = match engine.model.new_context(&engine.backend, ctx_params) {
            Ok(c) => c,
            Err(_) => return String::new(),
        };

        let mut tokens_list = engine.model.str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .unwrap_or_default();

        let safe_max = if max_tokens > n_ctx_limit as usize { (n_ctx_limit / 2) as usize } else { max_tokens };
        let max_prompt_len = (n_ctx_limit as usize).saturating_sub(safe_max).saturating_sub(10);
        
        if tokens_list.len() > max_prompt_len {
            let excess = tokens_list.len() - max_prompt_len;
            let start_drain = 50.min(tokens_list.len() / 2);
            let end_drain = (start_drain + excess).min(tokens_list.len().saturating_sub(20));

            if start_drain < end_drain {
                tokens_list.drain(start_drain..end_drain);
            } else {
                tokens_list.truncate(max_prompt_len);
            }
        }

        if tokens_list.is_empty() { 
            return String::new(); 
        }

        println!("[LLM] Ingesting prompt ({} tokens)...", tokens_list.len());
        
        let mut batch = llama_cpp_2::llama_batch::LlamaBatch::new(n_batch_limit as usize, 1);
        let mut n_cur = 0;

        for chunk in tokens_list.chunks(n_batch_limit as usize) {
            batch.clear();
            let is_last_chunk = n_cur + chunk.len() == tokens_list.len();
            
            for (i, &token) in chunk.iter().enumerate() {
                let is_last_token = is_last_chunk && (i == chunk.len() - 1);
                if batch.add(token, (n_cur + i) as i32, &[0], is_last_token).is_err() {
                    return String::new();
                }
            }
            
            if ctx.decode(&mut batch).is_err() {
                return String::new();
            }
            n_cur += chunk.len();
            print!(".");
            let _ = std::io::stdout().flush();
        }
        
        println!(" [Done]");
        print!("[LLM] Generating: ");
        let _ = std::io::stdout().flush();

        let mut output = String::new();
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let absolute_max = (tokens_list.len() + max_tokens).min(n_ctx_limit as usize);

        while n_cur < absolute_max {
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
            
            print!("{}", token_str);
            let _ = std::io::stdout().flush();

            if output.contains("<|end|>") || output.contains("<|user|>") || output.contains("<|assistant|>") {
                break;
            }

            batch.clear();
            if batch.add(best_token, n_cur as i32, &[0], true).is_err() {
                break;
            }
            
            if ctx.decode(&mut batch).is_err() {
                break;
            }
            n_cur += 1;
        }

        println!("\n[LLM] Request complete.");
        output
    }

    pub fn determine_intent(query: &str, explicit_synthesis: bool) -> LlmIntent {
        if explicit_synthesis { return LlmIntent::SynthesizeAnswer; }

        let lower = query.to_lowercase();
        
        let filter_triggers = [
            "less than", "greater than", "more than", "under ", "over ", 
            "below ", "above ", "without ", "exactly ", "minder dan", "meer dan", "contain"
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
                if start < end {
                    let json_str = &response[start..=end];
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(min) = parsed["min_ts"].as_u64() { query.min_timestamp = Some(min); }
                        if let Some(max) = parsed["max_ts"].as_u64() { query.max_timestamp = Some(max); }
                        if let Some(clean) = parsed["clean_query"].as_str() { query.raw_text = clean.to_string(); }
                    }
                }
            }
        }
    }

    /// Generalized Semantic Extractor
    /// Scans the document for the densest cluster of the user's query terms and extracts a clean window of surrounding context.
    fn extract_relevant_window(text: &str, condition: &str, window_chars: usize) -> String {
        let stop_words = ["what", "how", "why", "who", "when", "the", "and", "for", "with", "that", "this", "are", "you", "from", "does", "was", "is", "a", "an", "of", "in", "to"];
        let lower_text = text.to_lowercase();
        
        let clean_cond: String = condition.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect();

        let query_terms: Vec<&str> = clean_cond.split_whitespace()
            .filter(|t| t.len() > 2 && !stop_words.contains(t))
            .collect();

        if query_terms.is_empty() {
            return text.chars().take(window_chars).collect();
        }

        let mut positions = Vec::new();
        for term in &query_terms {
            let mut start = 0;
            // .find() is guaranteed to return valid char boundaries
            while let Some(pos) = lower_text[start..].find(term) {
                let absolute_pos = start + pos;
                positions.push(absolute_pos);
                start = absolute_pos + term.len();
            }
        }

        if positions.is_empty() {
            return text.chars().take(window_chars).collect();
        }

        // Find the densest cluster of terms within a given approximate span
        positions.sort_unstable();
        let mut best_byte_start = positions[0];
        let mut max_density = 0;
        
        let window_bytes = window_chars * 2; 

        for i in 0..positions.len() {
            let start_pos = positions[i];
            let end_pos = start_pos + window_bytes;
            let mut count = 0;

            for j in i..positions.len() {
                if positions[j] < end_pos {
                    count += 1;
                } else {
                    break;
                }
            }

            if count > max_density {
                max_density = count;
                best_byte_start = start_pos;
            }
        }

        // Translate safe byte boundary back to an absolute character index for multi-byte resilient slicing
        let start_char_idx = text[..best_byte_start].chars().count();
        let safe_start = start_char_idx.saturating_sub(60); // Provide 60 chars of leading context
        
        text.chars().skip(safe_start).take(window_chars).collect()
    }

    pub fn filter_with_llm(&self, condition: &str, mut candidates: Vec<SearchResult>) -> Vec<SearchResult> {
        if candidates.is_empty() { return vec![]; }

        candidates.truncate(5);
        let mut docs_block = String::new();
        
        for (i, doc) in candidates.iter().enumerate() {
            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
            let safe_content = Self::extract_relevant_window(content, condition, 400); 
            docs_block.push_str(&format!("[ID: {}]\n{}\n\n", i, safe_content));
        }

        let prompt = format!(
            "<|user|>\nDetermine which documents satisfy this condition: \"{}\". \
            Review the following document excerpts. Return ONLY a JSON array of the matching [ID]s (e.g., [0, 2]). If none match, return [].\n\n\
            DOCUMENTS:\n{}<|end|>\n<|assistant|>\n",
            condition, docs_block
        );

        let response = self.generate_text(&prompt, 150);
        let mut matched_indices: Vec<usize> = Vec::new();
        
        if let Some(start) = response.find('[') {
            let mut open_brackets = 0;
            let mut end_offset = 0;
            let slice = &response[start..];
            
            for (i, c) in slice.char_indices() {
                if c == '[' { 
                    open_brackets += 1; 
                } else if c == ']' { 
                    open_brackets -= 1; 
                    if open_brackets == 0 {
                        end_offset = i;
                        break;
                    }
                }
            }
            
            if end_offset > 0 {
                let json_str = &slice[..=end_offset];
                if let Ok(parsed) = serde_json::from_str::<Vec<usize>>(json_str) {
                    matched_indices = parsed;
                }
            }
        }

        candidates.into_iter().enumerate()
            .filter(|(i, _)| matched_indices.contains(i))
            .map(|(_, doc)| doc)
            .collect()
    }

    pub fn generate_synthesis(&self, query: &str, mut context_docs: Vec<SearchResult>) -> String {
        if context_docs.is_empty() { return "No documents found.".to_string(); }

        context_docs.truncate(4);
        
        let mut context_block = String::new();
        for (i, doc) in context_docs.iter().enumerate() {
            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
            let safe_content = Self::extract_relevant_window(content, query, 600);
            // INJECT: Use doc.title so the prompt sees the file name
            context_block.push_str(&format!("Source [{}] ({}):\n{}\n\n", i + 1, doc.title, safe_content));
        }

        // INJECT: Strong prompt engineering for citations and confidence metric
        let prompt = format!(
            "<|user|>\nYou are an analytical AI assistant. Answer the user's query using ONLY the provided context documents. \
            \n\nCRITICAL INSTRUCTIONS:\
            \n1. You MUST explicitly cite the Source name (e.g., 'According to pie2.png...') for every fact you provide.\
            \n2. At the very end of your response, you MUST provide a 'Confidence Score: X%' and a brief 'Confidence Interval Justification'. \
            Score 100% ONLY if the provided text contains the complete, explicit answer. Score lower if you had to infer, or if the data is partial/fragmented. \
            \nIf the answer is not contained in the documents at all, state that you do not have enough information and give a Confidence Score of 0%.\n\n\
            CONTEXT:\n{}\n\n\
            QUERY: {}<|end|>\n<|assistant|>\n",
            context_block, query
        );

        self.generate_text(&prompt, 500)
    }
}