// src/engine/llm/mod.rs

pub mod strategy_filter;
pub mod strategy_intent;
pub mod strategy_temporal;
pub mod strategy_synthesis;
pub mod strategy_script;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::io::Write;
use std::thread;

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;

use crate::domain::{SearchQuery, SearchResult};
use crate::engine::model_manager::ModelManager;
use crate::engine::HardwareManager;

pub use strategy_filter::FastFilterStrategy;
pub use strategy_intent::{LlmIntent, IntentStrategy};
pub use strategy_temporal::TemporalStrategy;
pub use strategy_synthesis::SynthesisStrategy;
pub use strategy_script::{ScriptCompilerStrategy, ScriptFixerStrategy, ScriptEvaluatorStrategy};

pub trait LlmStrategy {
    type Input;
    type Output;
    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output;
}

pub struct LlmCore {
    pub backend: LlamaBackend,
    pub model: LlamaModel,
    pub supports_cot: bool,
}

impl LlmCore {
    pub fn generate_text(&self, strategy_name: &str, prompt: &str, max_tokens: usize, is_cancelled: Arc<AtomicBool>) -> String {
        println!("\n=======================================================================================");
        println!("[DEBUG] STRATEGY: [{}]", strategy_name.to_uppercase());
        println!("[DEBUG] RAW TEXT/PROMPT FED TO THE LLM:");
        println!("---------------------------------------------------------------------------------------");
        println!("{}", prompt);
        println!("=======================================================================================\n");

        let n_ctx_limit: u32 = 4096;
        let n_batch_limit: u32 = 512;
        
        let optimal_threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8) as i32)
            .unwrap_or(4);

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

        let adjusted_max_tokens = max_tokens + 1024;
        let safe_max = if adjusted_max_tokens > n_ctx_limit as usize { (n_ctx_limit / 2) as usize } else { adjusted_max_tokens };
        let max_prompt_len = (n_ctx_limit as usize).saturating_sub(safe_max).saturating_sub(10);
        
        if tokens_list.len() > max_prompt_len {
            println!("[LLM Warning] Prompt length ({} tokens) exceeds limit. Truncating tail to fit context window safely.", tokens_list.len());
            tokens_list.truncate(max_prompt_len);
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
        let absolute_max = (tokens_list.len() + adjusted_max_tokens).min(n_ctx_limit as usize);

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

            if token_str.contains("<|end|>") 
                || token_str.contains("<|user|>") 
                || token_str.contains("<|assistant|>") 
                || token_str.contains("<|eot_id|>") 
                || token_str.contains("<|im_end|>") 
                || token_str.contains("<|im_start|>")
                || token_str.contains("<|endoftext|>")
                || token_str.contains("</s>")
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
        
        let mut final_output = output;
        
        // Safe <think> Tag Stripper
        while let Some(start_idx) = final_output.find("<think>") {
            if let Some(end_idx) = final_output.find("</think>") {
                let before = &final_output[..start_idx];
                let after = &final_output[end_idx + 8..];
                final_output = format!("{}{}", before, after);
            } else {
                let before = &final_output[..start_idx];
                let after = &final_output[start_idx + 7..];
                final_output = format!("{}{}", before, after);
                break;
            }
        }
        
        if let Some(end_idx) = final_output.find("</think>") {
            final_output = final_output[end_idx + 8..].to_string();
        }
        
        final_output = final_output.trim().to_string();

        println!("\n=======================================================================================");
        println!("[DEBUG] STRATEGY: [{}]", strategy_name.to_uppercase());
        println!("[DEBUG] RAW GENERATED LLM RESPONSE:");
        println!("---------------------------------------------------------------------------------------");
        println!("{}", final_output);
        println!("=======================================================================================\n");

        final_output
    }
}

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
        let safe_head = text.chars().take(window_chars).collect::<String>();
        return format!("{}...[TRUNCATED]", safe_head);
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

    let mut safe_byte_start = best_byte_start.saturating_sub(150);
    while safe_byte_start > 0 && !text.is_char_boundary(safe_byte_start) {
        safe_byte_start -= 1;
    }
    
    let extracted: String = text[safe_byte_start..].chars().take(window_chars).collect();
    
    if safe_byte_start == 0 {
        format!("{}...", extracted)
    } else {
        format!("...{}...", extracted)
    }
}

pub struct LlmService {
    engine: Arc<Mutex<Option<LlmCore>>>,
    pub boot_status: Arc<Mutex<String>>,
}

impl LlmService {
    pub fn new() -> Self {
        let engine = Arc::new(Mutex::new(None));
        let engine_clone = Arc::clone(&engine);
        
        // Track background progress dynamically for UI queries
        let boot_status = Arc::new(Mutex::new("Initializing background routines...".to_string()));
        let boot_status_clone = Arc::clone(&boot_status);

        // Spawn non-blocking thread to allow instant boot initialization
        thread::spawn(move || {
            println!("[LLM] Starting asynchronous background loading sequence...");
            let (model_path, model_url, supports_cot) = ModelManager::get_active_model_details();
            
            if let Ok(mut guard) = boot_status_clone.lock() {
                *guard = "Checking local model cache...".to_string();
            }

            // Safe execution of any blocking model downloads/verifications outside of main thread.
            // Passes the global status tracker so it can update the UI with percentage logs.
            ModelManager::ensure_model_available(&model_path, &model_url, Some(Arc::clone(&boot_status_clone)));

            let backend = match LlamaBackend::init() {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("[LLM Error] Failed to initialize backend context: {:?}", e);
                    if let Ok(mut guard) = boot_status_clone.lock() {
                        *guard = format!("Failed to initialize Llama backend: {:?}", e);
                    }
                    return;
                }
            };

            let n_gpu = HardwareManager::get_optimal_gpu_layers();
            let model_params = LlamaModelParams::default().with_n_gpu_layers(n_gpu);

            if let Ok(mut guard) = boot_status_clone.lock() {
                *guard = "Loading AI model into RAM/VRAM...".to_string();
            }

            match LlamaModel::load_from_file(&backend, &model_path, &model_params) {
                Ok(model) => {
                    let mut guard = engine_clone.lock().unwrap();
                    *guard = Some(LlmCore { backend, model, supports_cot });
                    
                    if let Ok(mut status_guard) = boot_status_clone.lock() {
                        *status_guard = "Ready".to_string();
                    }
                    println!("[LLM] Asynchronous background model loading complete. AI functionality activated!");
                }
                Err(e) => {
                    eprintln!("[LLM Error] Failed to load model in background thread: {:?}", e);
                    if let Ok(mut guard) = boot_status_clone.lock() {
                        *guard = format!("Failed to load model file: {:?}", e);
                    }
                }
            }
        });

        Self { engine, boot_status }
    }

    pub fn is_ready(&self) -> bool {
        self.engine.lock().unwrap().is_some()
    }

    pub fn switch_model<F>(&self, model_id: &str, send_chunk: &mut F, is_cancelled: Arc<AtomicBool>) -> Result<(), String> 
    where F: FnMut(String) 
    {
        let model_path = ModelManager::download_model_if_needed(model_id, send_chunk, is_cancelled)?;
        send_chunk(serde_json::json!({"status": "processing", "message": "Loading model into memory..."}).to_string());
        
        let parsed = ModelManager::setup_model_config();
        let supports_cot = parsed["models"][model_id]["supports_cot"]
            .as_bool()
            .unwrap_or_else(|| model_id.to_lowercase().contains("qwen"));
            
        let mut engine_guard = self.engine.lock().unwrap();
        let n_gpu = HardwareManager::get_optimal_gpu_layers();
        
        let backend = LlamaBackend::init().map_err(|_| "Failed to initialize LlamaBackend context")?;
        
        let new_model = LlamaModel::load_from_file(&backend, &model_path, &LlamaModelParams::default().with_n_gpu_layers(n_gpu))
            .map_err(|_| "Corrupted model file or failed to load into RAM")?;
        
        ModelManager::set_active_model(model_id)?;
        *engine_guard = Some(LlmCore {
            backend,
            model: new_model,
            supports_cot,
        });

        Ok(())
    }

    pub fn determine_intent(&self, query: &str, explicit_synthesis: bool, enable_ai_filtering: bool, is_cancelled: Arc<AtomicBool>) -> LlmIntent {
        if explicit_synthesis { return LlmIntent::SynthesizeAnswer; }

        let core_guard = self.engine.lock().unwrap();
        if let Some(ref core) = *core_guard {
            let strategy = IntentStrategy;
            strategy.execute(core, (query.to_string(), enable_ai_filtering), is_cancelled)
        } else {
            // Graceful safe fallback to direct keyword match when background initialization is running
            LlmIntent::Skip
        }
    }

    pub fn generate_synthesis(&self, query: &str, core_concept: &str, context_docs: Vec<SearchResult>, is_cancelled: Arc<AtomicBool>) -> serde_json::Value {
        let core_guard = self.engine.lock().unwrap();
        if let Some(ref core) = *core_guard {
            let strategy = SynthesisStrategy;
            strategy.execute(core, (query.to_string(), core_concept.to_string(), context_docs), is_cancelled)
        } else {
            let status_msg = self.boot_status.lock().unwrap().clone();
            serde_json::json!({
                "answer": format!("The AI search engine core is currently offline.\n\n**Status:** {}", status_msg),
                "reasoning": "Model background download or initialization sequence in progress.",
                "confidence_score": 0,
                "confidence_justification": "AI core unavailable",
                "cited_indices": []
            })
        }
    }

    pub fn compile_query_to_script(&self, query: &str, schema_keys: Vec<String>, is_cancelled: Arc<AtomicBool>) -> String {
        let core_guard = self.engine.lock().unwrap();
        if let Some(ref core) = *core_guard {
            let strategy = ScriptCompilerStrategy;
            strategy.execute(core, (query.to_string(), schema_keys), is_cancelled)
        } else {
            String::new()
        }
    }

    pub fn fix_script_syntax(&self, query: &str, broken_script: &str, error_msg: &str, schema_keys: Vec<String>, is_cancelled: Arc<AtomicBool>) -> String {
        let core_guard = self.engine.lock().unwrap();
        if let Some(ref core) = *core_guard {
            let strategy = ScriptFixerStrategy;
            strategy.execute(core, (query.to_string(), broken_script.to_string(), error_msg.to_string(), schema_keys), is_cancelled)
        } else {
            broken_script.to_string()
        }
    }

    pub fn evaluate_script_logic(&self, query: &str, compiled_script: &str, schema_keys: Vec<String>, is_cancelled: Arc<AtomicBool>) -> String {
        let core_guard = self.engine.lock().unwrap();
        if let Some(ref core) = *core_guard {
            let strategy = ScriptEvaluatorStrategy;
            strategy.execute(core, (query.to_string(), compiled_script.to_string(), schema_keys), is_cancelled)
        } else {
            "APPROVE".to_string()
        }
    }

    pub fn apply_temporal_heuristics(&self, query: &mut SearchQuery, is_cancelled: Arc<AtomicBool>) {
        let core_guard = self.engine.lock().unwrap();
        if let Some(ref core) = *core_guard {
            let strategy = TemporalStrategy;
            let (min, max, clean) = strategy.execute(core, query.raw_text.clone(), is_cancelled);
            
            if min.is_some() { query.min_timestamp = min; }
            if max.is_some() { query.max_timestamp = max; }
            if !clean.is_empty() { query.raw_text = clean; }
        }
    }

    #[allow(dead_code)]
    pub fn apply_fast_filter(&self, condition: &str, candidates: Vec<SearchResult>, is_cancelled: Arc<AtomicBool>) -> Vec<SearchResult> {
        let core_guard = self.engine.lock().unwrap();
        if let Some(ref core) = *core_guard {
            let strategy = FastFilterStrategy;
            strategy.execute(core, (condition.to_string(), candidates), is_cancelled)
        } else {
            candidates
        }
    }

    pub fn extract_core_concept(&self, query: &str, is_cancelled: Arc<AtomicBool>) -> String {
        let core_guard = self.engine.lock().unwrap();
        if let Some(ref core) = *core_guard {
            let cot_bypass = if core.supports_cot { "<think>\n</think>\n" } else { "" };
            
            let prompt = format!(
                "<|im_start|>system\nYou are a search engine keyword extractor. Extract ONLY the core factual subject nouns from the user's query. Ignore all conversational words, verbs, and question phrasing. Output ONLY space-separated keywords.<|im_end|>\n\
                <|im_start|>user\n\
                Query: \"{}\"\n\
                <|im_end|>\n\
                <|im_start|>assistant\n{}Keywords: ", query, cot_bypass
            );
            let response = core.generate_text("KEYWORD_EXTRACTION", &prompt, 150, is_cancelled);
            response.replace("Keywords:", "").trim().to_string()
        } else {
            query.to_string()
        }
    }
}