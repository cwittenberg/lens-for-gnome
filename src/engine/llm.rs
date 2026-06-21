// src/engine/llm.rs
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::io::Write;

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;

use crate::domain::{SearchQuery, SearchResult};
use crate::engine::model_manager::ModelManager;
use crate::engine::HardwareManager;

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
}

impl LlmService {
    pub fn new() -> Self {
        println!("Initializing llama.cpp Backend...");
        
        let backend = LlamaBackend::init().expect("Failed to initialize C++ backend");
        let (model_path, model_url) = ModelManager::get_active_model_path_and_url();
        
        ModelManager::ensure_model_available(&model_path, &model_url);

        let n_gpu = HardwareManager::get_optimal_gpu_layers();
        
        println!("[LLM] Note: If 'token_embd.weight' is mapped to the CPU in the following logs, this is EXPECTED.");
        println!("[LLM] The embedding lookup table stays in system RAM for fast O(1) lookups, while all compute layers offload to the GPU.");
        
        let model_params = LlamaModelParams::default().with_n_gpu_layers(n_gpu);
        
        let model = LlamaModel::load_from_file(&backend, &model_path, &model_params)
            .unwrap_or_else(|_| panic!("Failed to load GGUF model from {}.", model_path));

        Self {
            engine: Arc::new(Mutex::new(LlmEngine { backend, model })),
        }
    }

    pub fn switch_model<F>(&self, model_id: &str, send_chunk: &mut F, is_cancelled: Arc<AtomicBool>) -> Result<(), String> 
    where F: FnMut(String) 
    {
        let model_path = ModelManager::download_model_if_needed(model_id, send_chunk, is_cancelled)?;

        send_chunk(serde_json::json!({"status": "processing", "message": "Loading model into memory..."}).to_string());
        
        let mut engine_guard = self.engine.lock().unwrap();
        
        let n_gpu = HardwareManager::get_optimal_gpu_layers();
        
        println!("[LLM] Note: If 'token_embd.weight' is mapped to the CPU in the following logs, this is EXPECTED.");
        println!("[LLM] The embedding lookup table stays in system RAM for fast O(1) lookups, while all compute layers offload to the GPU.");
        
        let new_model = LlamaModel::load_from_file(&engine_guard.backend, &model_path, &LlamaModelParams::default().with_n_gpu_layers(n_gpu))
            .map_err(|_| "Corrupted model file or failed to load into RAM")?;
        
        ModelManager::set_active_model(model_id)?;
        
        engine_guard.model = new_model;

        Ok(())
    }

    fn extract_json(text: &str) -> Option<String> {
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        if start <= end {
            Some(text[start..=end].to_string())
        } else {
            None
        }
    }

    fn generate_text(&self, prompt: &str, max_tokens: usize, is_cancelled: Arc<AtomicBool>) -> String {
        println!("\n=======================================================================================");
        println!("[DEBUG] RAW TEXT/PROMPT FED TO THE LLM:");
        println!("---------------------------------------------------------------------------------------");
        println!("{}", prompt);
        println!("=======================================================================================\n");

        let engine = match self.engine.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        
        let n_ctx_limit: u32 = 4096;
        let n_batch_limit: u32 = 512;
        
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
            
            // Protect the first 500 tokens (System Instructions) from being chopped
            let start_drain = 500.min(tokens_list.len() / 2);
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
            if is_cancelled.load(Ordering::Relaxed) {
                println!("\n[LLM] Request cancelled by client during ingestion.");
                return String::new();
            }

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
            if is_cancelled.load(Ordering::Relaxed) {
                println!("\n[LLM] Request cancelled by client during generation.");
                break;
            }

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

            // 'special' flag set to true so stop tokens like <|endoftext|> resolve to strings and can be caught
            let token_str = engine.model.token_to_piece(best_token, &mut decoder, true, None)
                .unwrap_or_default();
                
            output.push_str(&token_str);
            
            print!("{}", token_str);
            let _ = std::io::stdout().flush();

            // Exhaustive stop condition to catch all base model EOG strings across architectures
            if output.contains("<|end|>") 
                || output.contains("<|user|>") 
                || output.contains("<|assistant|>") 
                || output.contains("<|eot_id|>") 
                || output.contains("<|im_end|>") 
                || output.contains("<|im_start|>")
                || output.contains("<|endoftext|>")
                || output.contains("</s>") 
            {
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

    pub fn determine_intent(&self, query: &str, explicit_synthesis: bool, is_cancelled: Arc<AtomicBool>) -> LlmIntent {
        if explicit_synthesis { return LlmIntent::SynthesizeAnswer; }

        let lower = query.to_lowercase();
        let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
        
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

        if !has_trigger {
            return LlmIntent::Skip;
        }

        let prompt = format!(
            "<|user|>\nYou are a strict multilingual routing API. Classify the user's intent for this search query into ONE category.\n\
            The query may be in any language (e.g. Spanish, Chinese, Dutch). Translate its semantic meaning internally before classifying.\n\
            Categories:\n\
            1: SKIP (Standard keyword search for documents, nouns, entities, or names)\n\
            2: REFINE_TIME (Query filters by relative time, e.g., 'last week', '3 days ago', 'hace 2 dias', 'vorige week')\n\
            3: FILTER_VALUE (Query asks to filter or evaluate documents based on a condition, e.g., 'find invoices below 100', 'greater than 50', 'under 100', 'below')\n\
            4: SYNTHESIZE (Query asks a question to be answered, e.g., 'how does', 'explain', 'what is', 'como funciona', 'wat is')\n\n\
            CRITICAL: If the user is asking to FILTER a set of documents by a condition, you MUST answer 3.\n\
            Query: \"{}\"\n\
            Return ONLY a valid JSON object. Format: {{\"intent\": digit}}.<|end|>\n<|assistant|>\n{{",
            query
        );

        let response = self.generate_text(&prompt, 15, is_cancelled).trim().to_string();
        let full_response = format!("{{{}", response);
        
        if let Some(json_str) = Self::extract_json(&full_response) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                if let Some(intent_num) = parsed["intent"].as_i64() {
                    return match intent_num {
                        2 => LlmIntent::RefineSearch,
                        3 => LlmIntent::FilterResults,
                        4 => LlmIntent::SynthesizeAnswer,
                        _ => LlmIntent::Skip,
                    };
                }
            }
        }
        
        LlmIntent::Skip
    }

    pub fn apply_temporal_heuristics(&self, query: &mut SearchQuery, is_cancelled: Arc<AtomicBool>) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let lower_text = query.raw_text.to_lowercase();

        let prompt = format!(
            "<|user|>\nYou are a multilingual JSON-only extraction API. The current UNIX timestamp is {}. \
            Analyze the following search query (which may be in any language) and extract the time constraints. \
            Return ONLY a raw JSON object with NO markdown formatting. \
            Format: {{\"min_ts\": number_or_null, \"max_ts\": number_or_null, \"clean_query\": \"string_without_time_words\"}} \
            Query: \"{}\"<|end|>\n<|assistant|>\n{{",
            now, lower_text
        );

        let response = self.generate_text(&prompt, 150, is_cancelled);
        let full_response = format!("{{{}", response);
        
        if let Some(json_str) = Self::extract_json(&full_response) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                if let Some(min) = parsed["min_ts"].as_u64() { query.min_timestamp = Some(min); }
                if let Some(max) = parsed["max_ts"].as_u64() { query.max_timestamp = Some(max); }
                if let Some(clean) = parsed["clean_query"].as_str() { query.raw_text = clean.to_string(); }
            }
        }
    }

    fn extract_relevant_window(text: &str, condition: &str, window_chars: usize) -> String {
        let stop_words = [
            "what", "how", "why", "who", "when", "the", "and", "for", "with", "that", "this", "are", "you", "from", "does", "was", "is", "a", "an", "of", "in", "to",
            "que", "como", "por", "quien", "cuando", "el", "la", "los", "las", "y", "para", "con", "eso", "esto", "son", "tu", "desde", "hace", "era", "es", "un", "una", "de", "en",
            "wat", "hoe", "waarom", "wie", "wanneer", "de", "het", "en", "voor", "met", "dat", "dit", "zijn", "jij", "van", "doet", "was", "is", "een", "in", "naar"
        ];
        let lower_text = text.to_lowercase();
        
        // DYNAMIC PRESERVATION: Do not truncate if the document fits cleanly. 
        // Truncating structured documents drops aggregates like "Total" placed at the bottom.
        if text.len() <= window_chars + 500 {
            return text.to_string();
        }

        let clean_cond: String = condition.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect();

        let query_terms: Vec<&str> = clean_cond.split_whitespace()
            .filter(|t| t.len() > 2 && !stop_words.contains(t))
            .collect();

        if query_terms.is_empty() {
            // SAFE TRUNCATION: If we must truncate, preserve Head + Tail.
            let half = window_chars / 2;
            let head = text.chars().take(half).collect::<String>();
            let tail: String = text.chars().rev().take(half).collect::<Vec<char>>().into_iter().rev().collect();
            return format!("{}\n\n...[TRUNCATED]...\n\n{}", head, tail);
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
            let half = window_chars / 2;
            let head = text.chars().take(half).collect::<String>();
            let tail: String = text.chars().rev().take(half).collect::<Vec<char>>().into_iter().rev().collect();
            return format!("{}\n\n...[TRUNCATED]...\n\n{}", head, tail);
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

    pub fn filter_with_llm(&self, condition: &str, candidates: Vec<SearchResult>, is_cancelled: Arc<AtomicBool>) -> Vec<SearchResult> {
        if candidates.is_empty() { return vec![]; }

        let mut all_processed = Vec::new();
        let max_batch_chars = 11_000; // Safely fits within 4096 tokens alongside prompt overhead

        let mut chunks: Vec<Vec<SearchResult>> = Vec::new();
        let mut current_chunk = Vec::new();
        let mut current_chars = 0;

        for doc in candidates.into_iter() {
            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
            let safe_content_len = Self::extract_relevant_window(content, condition, 1200).len();
            let estimated_len = safe_content_len + 100; // Include delineator overhead

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
                let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
                let safe_content = Self::extract_relevant_window(content, condition, 1200); 
                docs_block.push_str(&format!("--- START DOCUMENT ID: {} ---\n{}\n--- END DOCUMENT ID: {} ---\n\n", i, safe_content, i));
            }

            let prompt = format!(
                "<|user|>\nDetermine which documents satisfy this condition (which may be in any language): \"{}\". \
                Review the following document excerpts. Evaluate each document completely independently of the others. Do not confuse numbers or values between documents.\n\
                CRITICAL INSTRUCTIONS:\n\
                1. You MUST output ONLY a valid JSON object. Do not include markdown formatting or backticks.\n\
                2. Be mathematically precise (e.g., 'below' means strictly '<', 'above' means strictly '>').\n\
                3. Keep 'reasoning' EXTREMELY minimal and brief (e.g., '180 > 100 -> FAIL' or '6 < 100 -> PASS') to save tokens.\n\
                Format:\n\
                {{\n\
                  \"evaluations\": [\n\
                    {{ \"id\": 0, \"reasoning\": \"<minimal logic>\", \"match\": true_or_false }}\n\
                  ]\n\
                }}\n\n\
                DOCUMENTS:\n{}<|end|>\n<|assistant|>\n{{",
                condition, docs_block
            );

            let response = self.generate_text(&prompt, 500, Arc::clone(&is_cancelled));
            let full_response = format!("{{{}", response);
            
            let mut matched_indices = Vec::new();
            let mut evaluations = std::collections::HashMap::new();

            if let Some(json_str) = Self::extract_json(&full_response) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    if let Some(evals) = parsed["evaluations"].as_array() {
                        for eval in evals {
                            let mut id_opt = eval["id"].as_u64();
                            if id_opt.is_none() {
                                if let Some(s) = eval["id"].as_str() {
                                    id_opt = s.parse::<u64>().ok();
                                }
                            }
                            if let Some(id) = id_opt {
                                let is_match = eval["match"].as_bool().unwrap_or(false);
                                let reasoning = eval["reasoning"].as_str().unwrap_or("").to_string();
                                if is_match {
                                    matched_indices.push(id as usize);
                                }
                                evaluations.insert(id as usize, (is_match, reasoning));
                            }
                        }
                    }
                }
            }

            for (i, doc) in chunk.iter_mut().enumerate() {
                if let Some((is_match, reasoning)) = evaluations.get(&i) {
                    doc.ai_matched = Some(*is_match);
                    doc.ai_reasoning = Some(reasoning.clone());
                } else {
                    let is_match = matched_indices.contains(&i);
                    doc.ai_matched = Some(is_match);
                    doc.ai_reasoning = Some(if is_match { "Condition met.".to_string() } else { "Condition not met.".to_string() });
                }
            }
            
            all_processed.extend(chunk.iter().cloned());
        }

        all_processed
    }

    pub fn generate_synthesis(&self, query: &str, mut context_docs: Vec<SearchResult>, is_cancelled: Arc<AtomicBool>) -> serde_json::Value {
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
            let content = doc.full_context.as_deref().unwrap_or(&doc.snippet);
            let safe_content = Self::extract_relevant_window(content, query, 1500);
            context_block.push_str(&format!("Source [{}] ({}):\n{}\n\n", i + 1, doc.title, safe_content));
        }

        let prompt = format!(
            "<|user|>\nYou are an analytical multilingual AI assistant. Answer the user's query using ONLY the provided context documents. \n\
            CRITICAL INSTRUCTIONS:\n\
            1. You MUST output ONLY a valid JSON object. Do not include markdown formatting or backticks.\n\
            2. You MUST evaluate the facts step-by-step in the 'reasoning' field BEFORE providing the final 'answer'.\n\
            3. You MUST explicitly cite the Source name (e.g., 'According to pie2.png...') in your answer.\n\
            Format:\n\
            {{\n\
              \"reasoning\": \"<extract values and evaluate conditions step-by-step>\",\n\
              \"answer\": \"<final comprehensive answer citing sources>\",\n\
              \"confidence_score\": 100,\n\
              \"confidence_justification\": \"<brief justification>\"\n\
            }}\n\n\
            CONTEXT:\n{}\n\n\
            QUERY: {}<|end|>\n<|assistant|>\n{{",
            context_block, query
        );

        let response = self.generate_text(&prompt, 1000, is_cancelled);
        let full_response = format!("{{{}", response);

        if let Some(json_str) = Self::extract_json(&full_response) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                return parsed;
            }
        }

        serde_json::json!({
            "answer": full_response,
            "reasoning": "Failed to parse structured JSON.",
            "confidence_score": 0,
            "confidence_justification": "Parsing Error"
        })
    }
}