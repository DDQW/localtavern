//! The chat engine and everything it needs: conversation sessions, streaming events, preset
//! resolution, reply post-processing and local image storage.

mod authoring;
pub mod compaction;
pub mod engine;
pub mod events;
mod extraction;
pub mod images;
pub mod meters;
mod next_voice;
mod overheard;
pub mod presets;
pub mod reply;
pub mod session;

#[cfg(test)]
mod engine_tests;

pub use authoring::LoreDraftRequest;
pub use engine::{ChatEngine, HolidayOverview, ImagePromptDraft, ToolDecision};
pub use events::{ChatEvent, EventSink};
pub use session::ChatSession;
