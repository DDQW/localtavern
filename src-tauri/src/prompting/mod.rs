//! Prompt construction: turns a conversation, its preset and loaded context into a provider-agnostic
//! [`LlmRequest`](crate::llm::LlmRequest), plus the tag/response parsers around it.

pub mod assembler;
pub mod context;
pub mod defaults;
pub mod emotion_tag;
pub mod notebook;
pub mod powershell_tool;
pub mod quest_log;
pub mod roles;
pub mod time_skip_tag;
pub mod world_info;
pub mod world_state;

pub use assembler::{apply_macros, assemble};
pub use context::PromptAssemblyContext;
pub use world_info::{KeywordWorldInfoScanner, WorldInfoMatch, WorldInfoScanner};
