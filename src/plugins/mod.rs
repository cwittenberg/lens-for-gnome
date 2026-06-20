// src/plugins/mod.rs
pub mod math;
pub mod email;
pub mod vector_search;

pub use math::MathPlugin;
pub use email::EmailPlugin;
pub use vector_search::VectorSearchPlugin;

use crate::domain::{SearchQuery, SearchResult};

pub trait PluginTool: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn can_fast_handle(&self, query: &SearchQuery) -> bool;
    fn execute(&self, query: &SearchQuery) -> Vec<SearchResult>;
}