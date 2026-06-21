// src/engine/llm/strategy_intent.rs
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use super::{LlmStrategy, LlmCore};

#[derive(PartialEq, Debug)]
pub enum LlmIntent {
    Skip,
    RefineSearch,
    SynthesizeAnswer,
    FilterResults,
}

pub struct IntentStrategy;

impl LlmStrategy for IntentStrategy {
    type Input = String;
    type Output = LlmIntent;

    fn execute(&self, core: &LlmCore, query: Self::Input, is_cancelled: Arc<AtomicBool>) -> Self::Output {
        let lower = query.to_lowercase();
        let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
        
        let synthesis_triggers = [
            "what", "how", "why", "who", "when", "where", "which", "explain", "summarize",
            "wat", "hoe", "waarom", "wie", "wanneer", 
            "warum", "wer", "wann", 
            "que", "como", "porque", "quien", "donde", "cual", "explique", "resuma"
        ];
        
        let filter_triggers = [
            "less", "greater", "under", "over", "below", "above", "only", "larger", "smaller", "exactly",
            "without", "excluding", "not", "contains", "containing", "exact",
            "minder", "meer", 
            "unter", "über", 
            "menos", "mayor", "debajo", "encima"
        ];
        
        let time_triggers = [
            "ago", "last", "past", "days", "weeks", "months", "years", "before", "after", "yesterday", "today",
            "geleden", "vorige", "laatste", "gisteren", "vandaag",
            "vor", "letzte", "gestern", "heute",
            "hace", "pasado", "dias", "semanas", "meses", "años", "ayer", "hoy"
        ];

        let mut has_trigger = false;
        if synthesis_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }
        if filter_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }
        if time_triggers.iter().any(|&w| words.contains(&w)) { has_trigger = true; }
        
        // Force evaluation if the user passed explicit string queries or quotes
        if query.contains('"') || query.contains('\'') { has_trigger = true; }

        if !has_trigger {
            return LlmIntent::Skip;
        }

        // Bracket encapsulation prevents quote collision.
        // Explicit PRIORITY hierarchy prevents SLMs from incorrectly weighting semantic filler words over math filters.
        let prompt = format!(
            "<|im_start|>system\nYou are a strict routing API. Output ONLY a single digit.<|im_end|>\n\
            <|im_start|>user\n\
            Classify the user's search intent into ONE digit:\n\
            1: SKIP (Standard keyword search)\n\
            2: REFINE_TIME (Time/Date filters, e.g., 'last year', 'yesterday')\n\
            3: FILTER_VALUE (Math/Logic/Exact filters, e.g., 'under 100', 'without', '\"exact\"')\n\
            4: SYNTHESIZE (Questions needing a written answer, e.g., 'explain how')\n\n\
            CRITICAL HIERARCHY OF RULES:\n\
            - PRIORITY A: If the query contains quantitative words ('less', 'greater', 'under', 'over') OR literal quotes (\") OR exclusionary words ('not', 'excluding', 'without'), you MUST answer 3.\n\
            - PRIORITY B: If it contains question words ('explain', 'what', 'how'), answer 4.\n\
            - PRIORITY C: Only if NO filters or questions exist, answer 1.\n\n\
            Query:\n\
            [{}]\n\
            <|im_end|>\n\
            <|im_start|>assistant\n\
            INTENT_DIGIT: ",
            query
        );

        let response = core.generate_text("INTENT_STRATEGY", &prompt, 5, is_cancelled).trim().to_string();
        let clean_response = response.replace("INTENT_DIGIT:", "");
        
        // Match 3 first since it's the highest execution priority
        if clean_response.contains('3') { return LlmIntent::FilterResults; }
        if clean_response.contains('2') { return LlmIntent::RefineSearch; }
        if clean_response.contains('4') { return LlmIntent::SynthesizeAnswer; }
        
        LlmIntent::Skip
    }
}