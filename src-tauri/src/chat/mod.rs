//! The chat engine and everything it needs: conversation sessions, streaming events, preset
//! resolution, reply post-processing and local image storage.

pub mod engine;
pub mod events;
mod extraction;
pub mod images;
pub mod presets;
pub mod reply;
pub mod session;

#[cfg(test)]
mod engine_tests;

pub use engine::{ChatEngine, ToolDecision};
pub use events::{ChatEvent, EventSink};
pub use session::ChatSession;
