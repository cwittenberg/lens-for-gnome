// src/plugins/math.rs
use crate::domain::{SearchQuery, SearchResult};
use crate::plugins::PluginTool;
use exmex::eval_str;
use std::collections::HashMap;

pub struct MathPlugin;

impl MathPlugin {
    fn is_safe_math_expression(expr: &str) -> bool {
        expr.chars().all(|c| {
            c.is_ascii_alphanumeric() 
            || c.is_ascii_whitespace()
            || matches!(c, '+' | '-' | '*' | '/' | '^' | '(' | ')' | '.' | ',')
        })
    }
}

impl PluginTool for MathPlugin {
    fn id(&self) -> &'static str { "plugin:math" }
    fn name(&self) -> &'static str { "Local Calculator" }
    
    fn can_fast_handle(&self, query: &SearchQuery) -> bool {
        query.raw_text.starts_with('=') || query.raw_text.starts_with("calc ")
    }
    
    fn execute(&self, query: &SearchQuery) -> Vec<SearchResult> {
        let expr = query.raw_text.replace("calc ", "").replace("=", "").trim().to_string();
        
        if !Self::is_safe_math_expression(&expr) {
            return vec![SearchResult {
                id: "math_security_block".to_string(),
                title: "Security Block".to_string(),
                snippet: "Expression contains illegal or unsafe characters.".to_string(),
                plugin_id: self.id().to_string(),
                score: 1.0,
                filename: None,
                filepath: None,
                metadata: HashMap::new(),
                created_at: None,
                indexed_at: None,
                full_context: None,
            }];
        }

        let (title, snippet, score) = match eval_str::<f64>(&expr) {
            Ok(val) => (format!("{} = {}", expr, val), "Calculated locally".to_string(), 1.0),
            Err(err) => ("Mathematical Syntax Error".to_string(), format!("{}", err), 0.8),
        };

        vec![SearchResult {
            id: format!("math_{}", expr),
            title,
            snippet,
            plugin_id: self.id().to_string(),
            score,
            filename: None,
            filepath: None,
            metadata: HashMap::new(),
            created_at: None,
            indexed_at: None,
            full_context: None,
        }]
    }
}