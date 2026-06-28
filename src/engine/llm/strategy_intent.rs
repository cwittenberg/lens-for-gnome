// src/engine/llm/strategy_intent.rs
// FIX: Removed assistant prefill; increased token limits for reasoning headroom.
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use super::{LlmStrategy, LlmCore};

#[derive(PartialEq, Debug)]
pub enum LlmIntent {
    Skip,
    RefineSearch,
    SynthesizeAnswer,
    FilterAst,
    FilterScript,
}

pub struct IntentStrategy;

impl LlmStrategy for IntentStrategy {
    type Input = (String, Option<String>);
    type Output = LlmIntent;

    fn execute(&self, core: &LlmCore, input: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let (query, filter_strategy) = input;
        
        let lower = query.to_lowercase();
        let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
        
        let synthesis_triggers = [
            "what", "how", "why", "who", "when", "where", "which", "explain", "summarize",
            "wat", "hoe", "waarom", "wie", "wanneer", 
            "warum", "wer", "wann", 
            "que", "como", "porque", "quien", "donde", "cual", "explique", "resuma"
        ];
        
        let ast_filter_triggers = [
            "less", "greater", "under", "over", "below", "above", "only", "larger", "smaller", "exactly",
            "without", "excluding", "not", "contains", "containing", "exact",
            "minder", "meer", 
            "unter", "über", 
            "menos", "mayor", "debajo", "encima"
        ];

        let script_filter_triggers = [
            "regex", "pattern", "starts", "ends", "starting", "ending", "format", "wildcard", "match"
        ];
        
        let time_triggers = [
            "ago", "last", "past", "days", "weeks", "months", "years", "before", "after", "yesterday", "today",
            "geleden", "vorige", "laatste", "gisteren", "vandaag",
            "vor", "letzte", "gestern", "heute",
            "hace", "pasado", "dias", "semanas", "meses", "años", "ayer", "hoy"
        ];

        let mut has_trigger = false;
        if synthesis_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }
        if ast_filter_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }
        if script_filter_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }
        if time_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }
        
        if query.contains('"') || query.contains('\'') { has_trigger = true; }

        let strat = filter_strategy.unwrap_or_else(|| "auto".to_string());
        
        if strat == "disabled" {
            if synthesis_triggers.iter().any(|&w| words.contains(&w)) {
            } else if time_triggers.iter().any(|&w| words.contains(&w)) {
            } else {
                return LlmIntent::Skip;
            }
        }

        if !has_trigger {
            return LlmIntent::Skip;
        }

        let prompt = format!(
            "<|im_start|>system\nYou are a strict routing API. Output ONLY a single digit. Thinking, reasoning, or dialogue are strictly FORBIDDEN.<|im_end|>\n\
            <|im_start|>user\n\
            Classify the user's search intent into ONE digit:\n\
            1: SKIP (Standard keyword search)\n\
            2: REFINE_TIME (Time/Date filters, e.g., 'last year', 'yesterday')\n\
            3: FILTER_AST (Basic math/logic/exact filters, e.g., 'under 100', 'without', '\"exact\"')\n\
            4: SYNTHESIZE (Questions needing a written descriptive answer, e.g., 'explain how', 'how much', 'what is', 'why')\n\
            5: FILTER_SCRIPT (Complex programmatic logic, Regex, 'starts with', 'ends with', complex substrings)\n\n\
            CRITICAL HIERARCHY OF RULES:\n\
            - PRIORITY A: If the query is a question asking for an explanation, summary, or descriptive answer (e.g., 'how much', 'what', 'how', 'why', 'who', 'when', 'where', 'explain', 'summarize'), answer 4.\n\
            - PRIORITY B: If the query requests regex, patterns, or complex string manipulation (starts/ends with, format), answer 5.\n\
            - PRIORITY C: If the query contains quantitative filters ('less', 'greater', 'under', 'over') OR literal quotes (\") OR exclusionary words ('not', 'excluding', 'without'), answer 5.\n\
            - PRIORITY D: Only if NO filters or questions exist, answer 1.\n\n\
            Query:\n\
            [{}]\n\
            <|im_end|>\n\
            <|im_start|>assistant\n<think>\n</think>\n",
            query
        );

        let response = core.generate_text("INTENT_STRATEGY", &prompt, 150, is_cancelled);
        
        let mut intent = LlmIntent::Skip;
        if response.contains('4') { intent = LlmIntent::SynthesizeAnswer; }
        else if response.contains('5') { intent = LlmIntent::FilterScript; }
        else if response.contains('3') { intent = LlmIntent::FilterScript; } 
        else if response.contains('2') { intent = LlmIntent::RefineSearch; }

        match strat.as_str() {
            "auto" => {
                if intent == LlmIntent::FilterAst { 
                    intent = LlmIntent::FilterScript; 
                }
            },
            "ast-only" => {
                if intent == LlmIntent::FilterScript { intent = LlmIntent::FilterAst; }
            },
            "script-only" => {
                if intent == LlmIntent::FilterAst { intent = LlmIntent::FilterScript; }
            },
            "disabled" => {
                if intent == LlmIntent::FilterAst || intent == LlmIntent::FilterScript {
                    intent = LlmIntent::Skip;
                }
            },
            _ => {}
        }

        intent
    }
}