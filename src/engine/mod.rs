// src/engine/mod.rs
pub mod llm;
pub mod model_manager;
pub mod router;
pub mod thread_pool;
pub mod vision;
pub mod smart_extract;
pub mod hardware;

pub use router::SystemRouter;
pub use thread_pool::ThreadPool;
pub use hardware::HardwareManager;