// src/engine/llm/strategy_ast.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use super::{LlmStrategy, LlmCore};

pub struct AstCompilerStrategy;

impl LlmStrategy for AstCompilerStrategy {
    type Input = (String, Vec<String>);
    type Output = serde_json::Value;

    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let (query, schema_keys) = input;
        
        let schema_str = if schema_keys.is_empty() {
            "No specific metadata fields available. Rely on SEARCH or CONTAINS.".to_string()
        } else {
            schema_keys.join(", ")
        };

        let prompt = format!(
            "<|im_start|>system\nYou are a strict AST Compiler. Convert queries into a compact LISP-JSON array syntax.<|im_end|>\n\
            <|im_start|>user\n\
            Available metadata fields discovered in user's database: [{}]\n\
            \n\
            Syntax:\n\
            [\"SEARCH\", \"semantic concept\"]\n\
            [\"CONTAINS\", \"exact_string\"]\n\
            [\"EQ\", \"field_name\", \"exact_value\"]\n\
            [\"NEQ\", \"field_name\", \"exact_value\"]\n\
            [\"GT\", \"field_name\", numeric_value]\n\
            [\"LT\", \"field_name\", numeric_value]\n\
            [\"AND\", [expr1], [expr2]]\n\
            [\"OR\", [expr1], [expr2]]\n\
            [\"NOT\", [expr]]\n\
            \n\
            Example Query: \"Contracts from John about hosting under 500\"\n\
            Example Output: [\"AND\", [\"SEARCH\", \"hosting\"], [\"EQ\", \"filetype\", \"contract\"], [\"EQ\", \"author\", \"John\"], [\"LT\", \"price\", 500]]\n\
            \n\
            Example Query: \"Invoices excluding zerospace-eu-ch-1\"\n\
            Example Output: [\"AND\", [\"SEARCH\", \"invoices\"], [\"NOT\", [\"CONTAINS\", \"zerospace-eu-ch-1\"]]]\n\
            \n\
            CRITICAL RULES:\n\
            1. Output ONLY a valid JSON array. No markdown, no text.\n\
            2. NEVER use field names that are not in the Available list. If a requested metadata field does not exist, use SEARCH instead.\n\
            3. IF A TERM IS WRAPPED IN QUOTES (e.g. \"exact\"), YOU MUST USE THE [\"CONTAINS\", \"exact\"] OPERATOR. DO NOT use SEARCH for quoted strings or explicit IDs.\n\
            4. Wrap any exclusionary conditions (not, without, excluding) in a NOT operator.\n\
            \n\
            Query: \"{}\"<|im_end|>\n\
            <|im_start|>assistant\n\
            [",
            schema_str, query
        );

        let response = core.generate_text("AST_COMPILER_STRATEGY", &prompt, 150, is_cancelled);
        let clean_resp = response.trim();
        
        let full_response = if clean_resp.starts_with('[') {
            clean_resp.to_string()
        } else {
            format!("[{}", clean_resp)
        };

        let mut json_str = if let Some(start) = full_response.find('[') {
            full_response[start..].to_string()
        } else {
            full_response.clone()
        };

        let mut parsed_val = serde_json::from_str::<serde_json::Value>(&json_str);
        
        while parsed_val.is_err() && json_str.len() > 2 {
            json_str.pop();
            parsed_val = serde_json::from_str::<serde_json::Value>(&json_str);
        }

        if let Ok(parsed) = parsed_val {
            return parsed;
        }

        serde_json::json!(["SEARCH", query])
    }
}