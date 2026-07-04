use std::collections::HashMap;

#[derive(Clone)]
pub struct CachedDoc {
    pub id: String,
    pub modified_at: u64,
    pub metadata: HashMap<String, String>,
    pub embedding: Vec<f32>,
}