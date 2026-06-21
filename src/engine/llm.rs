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
use crate::engine::model_manager::ModelManager;

#[derive(PartialEq)]
pub enum LlmIntent {
    Skip,
    RefineSearch,
    SynthesizeAnswer,
    FilterResults,
}

pub struct LlmEngine {
    pub backend: LlamaBackend,
    pub model: LlamaModel,
}

pub struct LlmService {
    engine: Arc<Mutex<LlmEngine>>,
    debug_mode: bool,
}

impl LlmService {
    pub fn new() -> Self {
        println!("Initializing llama.cpp Backend...");
        
        let backend = LlamaBackend::init().expect("Failed to initialize C++ backend");
        let (model_path, model_url) = ModelManager::get_active_model_path_and_url();
        
        ModelManager::ensure_model_available(&model_path, &model_url);

        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(&backend, &model_path, &model_params)
            .unwrap_or_else(|_| panic!("Failed to load GGUF model from {}.", model_path));

        let debug_mode = env::var("DEBUG_LLM_PROMPT").unwrap_or_else(|_| "0".to_string()) == "1";
        
        Self {
            engine: Arc::new(Mutex::new(LlmEngine { backend, model })),
            debug_mode,
        }
    }

    /// Hotswaps the AI model. Yields execution to the manager to fetch missing files.
    pub fn switch_model<F>(&self, model_id: &str, send_chunk: &mut F) -> Result<(), String> 
    where F: FnMut(String) 
    {
        let model_path = ModelManager::download_model_if_needed(model_id, send_chunk)?;

        send_chunk(serde_json::json!({"status": "processing", "message": "Loading model into memory..."}).to_string());
        
        // Block inferences and load the newly requested model
        let mut engine_guard = self.engine.lock().unwrap();
        let new_model = LlamaModel::load_from_file(&engine_guard.backend, &model_path, &LlamaModelParams::default())
            .map_err(|_| "Corrupted model file or failed to load into RAM")?;
        
        ModelManager::set_active_model(model_id)?;
        
        engine_guard.model = new_model;

        Ok(())
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
        
        // Dynamically allocate CPU threads, maxing out at 8. 
        // Cast to i32 as required by llama_cpp_2 context parameters.
        let available_threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4);
        let optimal_threads = available_threads.min(8);
        
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(std::num::NonZeroU32::new(n_ctx_limit))
            .with_n_batch(n_batch_limit)
            .with_n_threads(optimal_threads)
            .with_n_threads_batch(optimal_threads);
            
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

        println!("[LLM] Ingesting prompt ({} tokens) on {} threads...", tokens_list.len(), optimal_threads);
        
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

            if output.contains("<|end|>") || output.contains("<|user|>") || output.contains("<|assistant|>") 
                || output.contains("<|eot_id|>") || output.contains("<|im_end|>") || output.contains("<|im_start|>") {
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

    /// Zero-Shot Semantic Intent Classifier gated by a Zero-Cost Lexical Pre-Filter.
    pub fn determine_intent(&self, query: &str, explicit_synthesis: bool) -> LlmIntent {
        if explicit_synthesis { return LlmIntent::SynthesizeAnswer; }

        let lower = query.to_lowercase();
        let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
        
        // ------------------------------------------------------------------------------------
        // FAST PRE-FILTER (Zero-Cost): Only engage the Neural Network if the query 
        // contains linguistic markers that suggest a question, filter, or temporal constraint.
        // ------------------------------------------------------------------------------------
        let trigger_words = [
            // English
            "what", "how", "why", "who", "when", "where", "which", "explain", "summarize",
            "less", "greater", "under", "over", "below", "above", "only", 
            "ago", "last", "past", "days", "weeks", "months", "years", "before", "after",
            // Dutch / German
            "wat", "hoe", "waarom", "wie", "wanneer", "minder", "meer", "geleden", "vorige", "laatste",
            "wie", "was", "warum", "wer", "wann", "unter", "über", "vor", "letzte",
            // Spanish
            "que", "como", "porque", "quien", "donde", "cual", "explique", "resuma",
            "menos", "mayor", "debajo", "encima", "hace", "pasado", "dias", "semanas", "meses", "años"
        ];

        let has_trigger = trigger_words.iter().any(|&w| words.contains(&w));

        // If the query is just standard keyword terms (e.g. "invoice 2026", "q3 report"), skip the LLM instantly.
        if !has_trigger {
            return LlmIntent::Skip;
        }

        // ------------------------------------------------------------------------------------
        // SEMANTIC VERIFICATION: If markers exist, ask the LLM to verify the semantic intent
        // ------------------------------------------------------------------------------------
        let prompt = format!(
            "<|user|>\nYou are a strict multilingual routing API. Classify the user's intent for this search query into ONE category.\n\
            The query may be in any language (e.g. Spanish, Chinese, Dutch). Translate its semantic meaning internally before classifying.\n\
            Categories:\n\
            1: SKIP (Standard keyword search for documents, nouns, entities, or names)\n\
            2: REFINE_TIME (Query filters by relative time, e.g., 'last week', '3 days ago', 'hace 2 dias', 'vorige week')\n\
            3: FILTER_VALUE (Query filters by numerical conditions, e.g., 'greater than 50', 'under 100', 'below', 'menos que')\n\
            4: SYNTHESIZE (Query asks a question to be answered, e.g., 'how does', 'explain', 'what is', 'como funciona', 'wat is')\n\n\
            Query: \"{}\"\n\
            Return ONLY a single digit (1, 2, 3, or 4).<|end|>\n<|assistant|>\n",
            query
        );

        // Limit generation to 5 tokens to guarantee sub-50ms execution speed for the router
        let response = self.generate_text(&prompt, 5).trim().to_string();
        
        if response.contains('4') {
            LlmIntent::SynthesizeAnswer
        } else if response.contains('3') {
            LlmIntent::FilterResults
        } else if response.contains('2') {
            LlmIntent::RefineSearch
        } else {
            LlmIntent::Skip
        }
    }

    pub fn apply_temporal_heuristics(&self, query: &mut SearchQuery) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let lower_text = query.raw_text.to_lowercase();

        let prompt = format!(
            "<|user|>\nYou are a multilingual JSON-only extraction API. The current UNIX timestamp is {}. \
            Analyze the following search query (which may be in any language) and extract the time constraints. \
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

    fn extract_relevant_window(text: &str, condition: &str, window_chars: usize) -> String {
        // Universal stop words covering English, Spanish, Dutch, and French to prevent over-filtering during localized FTS
        let stop_words = [
            "what", "how", "why", "who", "when", "the", "and", "for", "with", "that", "this", "are", "you", "from", "does", "was", "is", "a", "an", "of", "in", "to",
            "que", "como", "por", "quien", "cuando", "el", "la", "los", "las", "y", "para", "con", "eso", "esto", "son", "tu", "desde", "hace", "era", "es", "un", "una", "de", "en",
            "wat", "hoe", "waarom", "wie", "wanneer", "de", "het", "en", "voor", "met", "dat", "dit", "zijn", "jij", "van", "doet", "was", "is", "een", "in", "naar"
        ];
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
            while let Some(pos) = lower_text[start..].find(term) {
                let absolute_pos = start + pos;
                positions.push(absolute_pos);
                start = absolute_pos + term.len();
            }
        }

        if positions.is_empty() {
            return text.chars().take(window_chars).collect();
        }

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

        let start_char_idx = text[..best_byte_start].chars().count();
        let safe_start = start_char_idx.saturating_sub(60);
        
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
            "<|user|>\nDetermine which documents satisfy this condition (which may be in any language): \"{}\". \
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
            context_block.push_str(&format!("Source [{}] ({}):\n{}\n\n", i + 1, doc.title, safe_content));
        }

        let prompt = format!(
            "<|user|>\nYou are an analytical multilingual AI assistant. Answer the user's query using ONLY the provided context documents. \
            You MUST reply in the same language as the user's query.\n\
            \nCRITICAL INSTRUCTIONS:\
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