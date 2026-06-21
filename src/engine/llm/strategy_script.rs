// src/engine/llm/strategy_script.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use super::{LlmStrategy, LlmCore};

pub struct ScriptCompilerStrategy;

impl LlmStrategy for ScriptCompilerStrategy {
    type Input = (String, Vec<String>);
    type Output = String;

    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let (query, schema_keys) = input;
        
        let schema_str = if schema_keys.is_empty() {
            "No specific metadata fields available.".to_string()
        } else {
            schema_keys.join(", ")
        };

        let prompt = format!(
            "<|im_start|>system\nYou are a query routing compiler. Map user queries to a single boolean Rhai (Rust) expression. Use the provided Standard Library functions. Output ONLY the raw script code.<|im_end|>\n\
            <|im_start|>user\n\
            Available metadata fields: [{}]\n\
            \n\
            STANDARD LIBRARY FUNCTIONS:\n\
            - `metadata`: A map of strings (e.g., metadata.filetype)\n\
            - `text`: The document content snippet.\n\
            - `title`: The document filename/title.\n\
            - `regex_match(string, pattern)`: Executes a fast regex search (e.g., `regex_match(text, \"(?i)invoice.*total\")`).\n\
            - `in_list(string, list)`: Checks if a string is in a comma-separated list (e.g., `in_list(metadata.filetype, \"pdf, docx, txt\")`).\n\
            - `days_ago(float)`: Returns the UNIX timestamp for N days ago.\n\
            - `parse_float(string)`: Safely converts a string to a number.\n\
            - `contains_ignore_case(string, search)`: Case-insensitive substring match.\n\
            \n\
            Example Query: \"PDF invoices ending in 2024 created in the last 14 days\"\n\
            Example Output:\n\
            let is_pdf = metadata.filetype == \"pdf\";\n\
            let mentions_invoice = contains_ignore_case(text, \"invoice\");\n\
            let ends_2024 = regex_match(text, \"2024$\");\n\
            let is_recent = parse_float(metadata.created_at) > days_ago(14.0);\n\
            is_pdf && mentions_invoice && ends_2024 && is_recent\n\
            \n\
            CRITICAL RULES:\n\
            1. Output ONLY valid Rhai script. No markdown formatting, no ```rhai blocks, no explanations.\n\
            2. The script MUST implicitly return a boolean value at the end.\n\
            3. NEVER invent or use metadata fields that are not explicitly listed in 'Available metadata fields'. If a field like 'price' or 'author' is not in the list, you MUST use regex_match on `text` instead to find it.\n\
            \n\
            Query: \"{}\"<|im_end|>\n\
            <|im_start|>assistant\n",
            schema_str, query
        );

        // Lower token limit enforces concise functional chaining instead of sprawling procedural logic
        let response = core.generate_text("SCRIPT_COMPILER_STRATEGY", &prompt, 150, is_cancelled);
        
        let mut clean_resp = response.trim().to_string();
        if clean_resp.starts_with("```rhai") {
            clean_resp = clean_resp.trim_start_matches("```rhai").trim().to_string();
        } else if clean_resp.starts_with("```") {
            clean_resp = clean_resp.trim_start_matches("```").trim().to_string();
        }
        if clean_resp.ends_with("```") {
            clean_resp = clean_resp.trim_end_matches("```").trim().to_string();
        }
        
        clean_resp
    }
}