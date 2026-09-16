//! # rc-llm
//!
//! LLM integration: Ollama and OpenAI-compatible backends, the merged model
//! registry behind `/api/models`, and the chat pipeline's payload/stream
//! machinery. The backend clients are designed as publishable, server-free
//! crates (DECISIONS D-011): they take base URLs and hold no DB state.

pub mod models;
pub mod ollama;
pub mod registry;

pub use models::ModelInfo;
