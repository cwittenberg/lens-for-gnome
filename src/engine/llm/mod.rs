// src/engine/llm/mod.rs
pub mod strategy_intent;
pub mod strategy_temporal;
pub mod strategy_filter;
pub mod strategy_ast;
pub mod strategy_synthesis;
pub mod strategy_script;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::io::Write;

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;

use crate::domain::{SearchQuery, SearchResult};
use crate::engine::model_manager::ModelManager;
use crate::engine::HardwareManager;

// Re-export strategies and types for the Router to consume
pub use strategy_intent::{LlmIntent, IntentStrategy};
pub use strategy_temporal::TemporalStrategy;
pub use strategy_filter::FastFilterStrategy;
pub use strategy_ast::AstCompilerStrategy;
pub use strategy_synthesis::SynthesisStrategy;
pub use strategy_script::ScriptCompilerStrategy;

// =====================================================================
// 1. STRATEGY INTERFACE
// =====================================================================

pub trait LlmStrategy {
    type Input;
    type Output;
    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output;
}

// =====================================================================
// 2. HARDWARE CORE (Inference Engine)
// =====================================================================

pub struct LlmCore {
    pub backend: LlamaBackend,
    pub model: LlamaModel,
}

impl LlmCore {
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

        Self { backend, model }
    }

    pub fn generate_text(&self, strategy_name: &str, prompt: &str, max_tokens: usize, is_cancelled: Arc<AtomicBool>) -> String {
        println!("\n=======================================================================================");
        println!("[DEBUG] STRATEGY: [{}]", strategy_name.to_uppercase());
        println!("[DEBUG] RAW TEXT/PROMPT FED TO THE LLM:");
        println!("---------------------------------------------------------------------------------------");
        println!("{}", prompt);
        println!("=======================================================================================\n");

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
            
        let mut ctx = match self.model.new_context(&self.backend, ctx_params) {
            Ok(c) => c,
            Err(_) => return String::new(),
        };

        let mut tokens_list = self.model.str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .unwrap_or_default();

        let safe_max = if max_tokens > n_ctx_limit as usize { (n_ctx_limit / 2) as usize } else { max_tokens };
        let max_prompt_len = (n_ctx_limit as usize).saturating_sub(safe_max).saturating_sub(10);
        
        if tokens_list.len() > max_prompt_len {
            let excess = tokens_list.len() - max_prompt_len;
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
            
            let mut best_token = self.model.token_eos();
            let mut max_logit = f32::NEG_INFINITY;

            for cand in candidates {
                if cand.logit() > max_logit {
                    max_logit = cand.logit();
                    best_token = cand.id();
                }
            }

            if best_token == self.model.token_eos() {
                break;
            }

            let token_str = self.model.token_to_piece(best_token, &mut decoder, true, None)
                .unwrap_or_default();
                
            output.push_str(&token_str);
            
            print!("{}", token_str);
            let _ = std::io::stdout().flush();

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
        
        println!("\n=======================================================================================");
        println!("[DEBUG] STRATEGY: [{}]", strategy_name.to_uppercase());
        println!("[DEBUG] RAW GENERATED LLM RESPONSE:");
        println!("---------------------------------------------------------------------------------------");
        println!("{}", output);
        println!("=======================================================================================\n");

        output
    }
}

// =====================================================================
// 3. SHARED DOMAIN UTILITIES
// =====================================================================

/// ARCHITECTURE CHANGE: Overhauled Semantic Sliding Window.
/// The previous iteration relied on exact matching, causing semantic mismatches to drop context entirely.
/// This implementation segments the text into broad semantic blocks and uses a robust term-frequency density 
/// check, falling back gracefully to the document head if no strong density is found, rather than cutting the 
/// text blindly.
pub fn extract_relevant_window(text: &str, condition: &str, window_chars: usize) -> String {
    let stop_words = [
        "what", "how", "why", "who", "when", "the", "and", "for", "with", "that", "this", "are", "you", "from", "does", "was", "is", "a", "an", "of", "in", "to",
        "que", "como", "por", "quien", "cuando", "el", "la", "los", "las", "y", "para", "con", "eso", "esto", "son", "tu", "desde", "hace", "era", "es", "un", "una", "de", "en",
        "wat", "hoe", "waarom", "wie", "wanneer", "de", "het", "en", "voor", "met", "dat", "dit", "zijn", "jij", "van", "doet", "was", "is", "een", "in", "naar"
    ];
    
    if text.len() <= window_chars + 500 {
        return text.to_string();
    }

    let lower_text = text.to_lowercase();
    let clean_cond: String = condition.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect();

    let query_terms: Vec<&str> = clean_cond.split_whitespace()
        .filter(|t| t.len() > 2 && !stop_words.contains(t))
        .collect();

    if query_terms.is_empty() {
        let safe_head = text.chars().take(window_chars).collect::<String>();
        return format!("{}...[TRUNCATED]", safe_head);
    }

    // Identify all byte-positions of query terms in the text
    let mut positions = Vec::new();
    for term in &query_terms {
        let mut start = 0;
        while let Some(pos) = lower_text[start..].find(term) {
            let absolute_pos = start + pos;
            positions.push(absolute_pos);
            start = absolute_pos + term.len();
        }
    }

    // Fallback if semantic intent drifted too far from exact keywords
    if positions.is_empty() {
        let safe_head = text.chars().take(window_chars).collect::<String>();
        return format!("{}...[TRUNCATED]", safe_head);
    }

    positions.sort_unstable();
    
    // Find the window with the highest density of term occurrences
    let mut best_byte_start = positions[0];
    let mut max_density = 0;
    let window_bytes = window_chars * 2; // Approximate byte width for utf-8 characters

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

    // Safely walk backwards to snap to the nearest word boundary
    let mut safe_byte_start = best_byte_start.saturating_sub(150);
    while safe_byte_start > 0 && !text.is_char_boundary(safe_byte_start) {
        safe_byte_start -= 1;
    }
    
    // Extract by character count safely to prevent slicing panics
    let extracted: String = text[safe_byte_start..].chars().take(window_chars).collect();
    
    if safe_byte_start == 0 {
        format!("{}...", extracted)
    } else {
        format!("...{}...", extracted)
    }
}

// =====================================================================
// 4. ORCHESTRATOR FACADE
// =====================================================================

pub struct LlmService {
    engine: Arc<Mutex<LlmCore>>,
}

impl LlmService {
    pub fn new() -> Self {
        Self {
            engine: Arc::new(Mutex::new(LlmCore::new())),
        }
    }

    pub fn switch_model<F>(&self, model_id: &str, send_chunk: &mut F, is_cancelled: Arc<AtomicBool>) -> Result<(), String> 
    where F: FnMut(String) 
    {
        let model_path = ModelManager::download_model_if_needed(model_id, send_chunk, is_cancelled)?;
        send_chunk(serde_json::json!({"status": "processing", "message": "Loading model into memory..."}).to_string());
        
        let mut engine_guard = self.engine.lock().unwrap();
        let n_gpu = HardwareManager::get_optimal_gpu_layers();
        
        let new_model = LlamaModel::load_from_file(&engine_guard.backend, &model_path, &LlamaModelParams::default().with_n_gpu_layers(n_gpu))
            .map_err(|_| "Corrupted model file or failed to load into RAM")?;
        
        ModelManager::set_active_model(model_id)?;
        engine_guard.model = new_model;
        Ok(())
    }

    pub fn determine_intent(&self, query: &str, explicit_synthesis: bool, filter_strategy: Option<String>, is_cancelled: Arc<AtomicBool>) -> LlmIntent {
        if explicit_synthesis { return LlmIntent::SynthesizeAnswer; }
        let strategy = IntentStrategy;
        let core = self.engine.lock().unwrap();
        strategy.execute(&core, (query.to_string(), filter_strategy), is_cancelled)
    }

    pub fn filter_with_llm(&self, condition: &str, candidates: Vec<SearchResult>, is_cancelled: Arc<AtomicBool>) -> Vec<SearchResult> {
        let strategy = FastFilterStrategy;
        let core = self.engine.lock().unwrap();
        strategy.execute(&core, (condition.to_string(), candidates), is_cancelled)
    }

    pub fn generate_synthesis(&self, query: &str, context_docs: Vec<SearchResult>, is_cancelled: Arc<AtomicBool>) -> serde_json::Value {
        let strategy = SynthesisStrategy;
        let core = self.engine.lock().unwrap();
        strategy.execute(&core, (query.to_string(), context_docs), is_cancelled)
    }

    pub fn compile_query_to_ast(&self, query: &str, schema_keys: Vec<String>, is_cancelled: Arc<AtomicBool>) -> serde_json::Value {
        let strategy = AstCompilerStrategy;
        let core = self.engine.lock().unwrap();
        strategy.execute(&core, (query.to_string(), schema_keys), is_cancelled)
    }
    
    pub fn compile_query_to_script(&self, query: &str, schema_keys: Vec<String>, is_cancelled: Arc<AtomicBool>) -> String {
        let strategy = ScriptCompilerStrategy;
        let core = self.engine.lock().unwrap();
        strategy.execute(&core, (query.to_string(), schema_keys), is_cancelled)
    }

    pub fn apply_temporal_heuristics(&self, query: &mut SearchQuery, is_cancelled: Arc<AtomicBool>) {
        let strategy = TemporalStrategy;
        let core = self.engine.lock().unwrap();
        let (min, max, clean) = strategy.execute(&core, query.raw_text.clone(), is_cancelled);
        
        if min.is_some() { query.min_timestamp = min; }
        if max.is_some() { query.max_timestamp = max; }
        if !clean.is_empty() { query.raw_text = clean; }
    }

    pub fn extract_core_concept(&self, query: &str, is_cancelled: Arc<AtomicBool>) -> String {
        let prompt = format!(
            "<|im_start|>system\nYou are a search engine keyword extractor. Extract ONLY the core factual subject nouns from the question. Ignore all conversational words, verbs, and question phrasing. Output ONLY space-separated keywords.<|im_end|>\n\
            <|im_start|>user\n\
            Question: \"what are the system requirements for installing ubuntu 24.04\"\n\
            Keywords: system requirements ubuntu 24.04 install\n\n\
            Question: \"how do i reset the administrator password on a cisco router\"\n\
            Keywords: reset administrator password cisco router\n\n\
            Question: \"{}\"\n\
            <|im_end|>\n\
            <|im_start|>assistant\n\
            Keywords: ", query
        );
        let core = self.engine.lock().unwrap();
        let response = core.generate_text("KEYWORD_EXTRACTION", &prompt, 20, is_cancelled);
        response.replace("Keywords:", "").trim().to_string()
    }
}