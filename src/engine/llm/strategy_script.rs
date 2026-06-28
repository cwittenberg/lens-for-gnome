// src/engine/llm/strategy_script.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use super::{LlmStrategy, LlmCore};

/// Helper to safely extract code when we prefill the prompt with ```rhai\n
fn extract_prefilled_rhai(response: &str) -> String {
    // The LLM was primed with ```rhai\n, so it starts outputting code immediately.
    // We just take everything before the first closing ```
    response.split("```").next().unwrap_or(response).trim().to_string()
}

fn extract_rhai_block(response: &str) -> String {
    let mut extracted = response.trim().to_string();
    if let Some(start_idx) = extracted.find("```rhai") {
        let content = &extracted[start_idx + 7..];
        if let Some(end_idx) = content.find("```") {
            extracted = content[..end_idx].trim().to_string();
        } else {
            extracted = content.trim().to_string();
        }
    } else if let Some(start_idx) = extracted.find("```") {
        let content = &extracted[start_idx + 3..];
        if let Some(end_idx) = content.find("```") {
            extracted = content[..end_idx].trim().to_string();
        } else {
            extracted = content.trim().to_string();
        }
    }
    extracted.trim().to_string()
}

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
            "<|im_start|>system\nYou are a high-speed query routing compiler. Map user queries to a single boolean Rhai (Rust) expression. You must output code immediately. <think> reasoning tags are strictly forbidden.<|im_end|>\n\
            <|im_start|>user\n\
            Available metadata fields: [{}]\n\
            \n\
            STANDARD LIBRARY FUNCTIONS:\n\
            - `metadata`: A map of strings (e.g., metadata.filetype)\n\
            - `text`: The document content snippet.\n\
            - `title`: The document filename/title.\n\
            - `regex_match(string, pattern)`: Executes a fast regex search.\n\
            - `regex_extract(string, pattern)`: Extracts the first regex match as a string.\n\
            - `in_list(string, list)`: Checks if a string is in a comma-separated list.\n\
            - `days_ago(float)`: Returns the UNIX timestamp for N days ago.\n\
            - `parse_float(string)`: Safely converts a string to a number.\n\
            - `contains_ignore_case(string, search)`: Case-insensitive substring match.\n\
            \n\
            Example Query: \"Receipts over 500\"\n\
            Example Output:\n\
            ```rhai\n\
            let is_receipt = contains_ignore_case(text, \"receipt\");\n\
            let amount_str = regex_extract(text, \"(?i)(?:total|amount|usd|\\\\$|cost)[^\\\\d]*(\\\\d+\\\\.?\\\\d*)\");\n\
            let amount_val = parse_float(amount_str);\n\
            is_receipt && amount_val > 500.0\n\
            ```\n\
            \n\
            CRITICAL RULES:\n\
            1. Output ONLY valid Rhai script.\n\
            2. The script MUST implicitly return a boolean value at the end.\n\
            3. DEFAULT TO AND: ALWAYS use `&&` (AND) to combine conditions unless the user explicitly uses \"or\".\n\
            4. USE QUERY VALUES: Extract the EXACT numbers and terms requested in the user's prompt.\n\
            5. ROBUST DYNAMIC EXTRACTION: When extracting numbers, dynamically generate context-relevant keywords for the non-capturing group based on the user's query domain. Pattern: `(?i)(?:<keyword1>|<keyword2>)[^\\\\d]*(\\\\d+\\\\.?\\\\d*)`.\n\
            6. TRANSLATE LOGIC: NEVER use quantitative words like 'more', 'less', 'under', 'over' inside a literal regex pattern. You MUST extract the targeted number using `parse_float(regex_extract(...))` and then apply mathematical operators (>, <, >=, <=) inside the Rhai script.\n\
            \n\
            Query: \"{}\"<|im_end|>\n\
            <|im_start|>assistant\n```rhai\n",
            schema_str, query
        );

        // We prefilled the prompt with ```rhai\n, trapping the LLM inside a code block.
        // It skips the <think> reasoning entirely and instantly generates the code.
        let response = core.generate_text("SCRIPT_COMPILER", &prompt, 1024, is_cancelled);
        extract_prefilled_rhai(&response)
    }
}

pub struct ScriptFixerStrategy;

impl LlmStrategy for ScriptFixerStrategy {
    type Input = (String, String, String, Vec<String>);
    type Output = String;

    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let (query, broken_script, error_msg, schema_keys) = input;
        
        let schema_str = if schema_keys.is_empty() {
            "No specific metadata fields available.".to_string()
        } else {
            schema_keys.join(", ")
        };

        let prompt = format!(
            "<|im_start|>system\nYou are a strict Rust/Rhai debugging expert. Fix the syntax errors and return the corrected script. <think> tags are strictly forbidden.<|im_end|>\n\
            <|im_start|>user\n\
            Original User Query: \"{}\"\n\
            Available metadata fields: [{}]\n\
            \n\
            BROKEN SCRIPT:\n\
            ```rhai\n\
            {}\n\
            ```\n\
            \n\
            COMPILER ERROR:\n\
            {}\n\
            \n\
            CRITICAL RULES:\n\
            1. Fix the error directly. Do not change the underlying logic if it is sound.\n\
            2. The script must return a boolean value.\n\
            3. Output ONLY the fixed script.<|im_end|>\n\
            <|im_start|>assistant\n```rhai\n",
            query, schema_str, broken_script, error_msg
        );

        let response = core.generate_text("SCRIPT_FIXER", &prompt, 1024, is_cancelled);
        extract_prefilled_rhai(&response)
    }
}

pub struct ScriptEvaluatorStrategy;

impl LlmStrategy for ScriptEvaluatorStrategy {
    type Input = (String, String, Vec<String>);
    type Output = String;

    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let (query, compiled_script, schema_keys) = input;
        
        let schema_str = if schema_keys.is_empty() {
            "No specific metadata fields available.".to_string()
        } else {
            schema_keys.join(", ")
        };

        let prompt = format!(
            "<|im_start|>system\nYou are a strict Quality Assurance reviewer evaluating an automated search script. Your job is to verify that the script logically matches the user's intent. <think> reasoning tags are strictly forbidden.<|im_end|>\n\
            <|im_start|>user\n\
            User Query: \"{}\"\n\
            Available metadata fields: [{}]\n\
            \n\
            COMPILED SCRIPT (Syntax is verified correct):\n\
            ```rhai\n\
            {}\n\
            ```\n\
            \n\
            REVIEW CHECKLIST:\n\
            1. Does the script accurately reflect mathematical bounds (e.g., if the user asked for 'under 100', does the script use `< 100` and NOT `> 100`)?\n\
            2. Does the script query hallucinated metadata fields that are NOT in the available fields list? (If a requested field doesn't exist, the script should use `regex_match(text, ...)` or `contains_ignore_case(text, ...)` instead of `metadata.fake_field`).\n\
            3. Is the logic sound for achieving the user's explicit goal?\n\
            \n\
            IF THE SCRIPT IS PERFECT: Output exactly the word \"APPROVE\".\n\
            IF THE SCRIPT HAS LOGICAL FLAWS: Output the corrected script in a ```rhai block.\n\
            DO NOT USE ANY REASONING OR TEXT OUTSIDE OF THESE EXACT COMMANDS.<|im_end|>\n\
            <|im_start|>assistant\n",
            query, schema_str, compiled_script
        );

        let response = core.generate_text("SCRIPT_EVALUATOR", &prompt, 1024, is_cancelled);
        let clean_resp = response.trim();
        
        // Match the raw output string without fuzzy extraction to prevent infinite failure loops
        if clean_resp.contains("APPROVE") && !clean_resp.contains("REJECT") && !clean_resp.contains("```") {
            return "APPROVE".to_string();
        }

        extract_rhai_block(clean_resp)
    }
}