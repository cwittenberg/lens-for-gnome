pub mod models;
pub mod security;
pub mod db;
pub mod search;
pub mod store;

// Facade: Re-export the primary structures so downstream 
// components (like the Router and Plugins) remain unbroken.
// We strictly export VectorStore, keeping internal models like 
// CachedDoc completely encapsulated within the vector boundary.
pub use store::VectorStore;