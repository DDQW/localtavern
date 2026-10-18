//! Repositories: plain functions over a `&Connection`, so they can be tested against an in-memory
//! database and wrapped by Tauri commands later. SQL, ordering and storage formats mirror the C#
//! repositories one-to-one so both apps can share a database file.

pub mod app_settings;
pub mod characters;
pub mod conversations;
pub mod group_chats;
pub mod lorebooks;
pub mod notebook;
pub mod personas;
pub mod prompt_presets;
pub mod quest_log;
pub mod summaries;
pub mod user_memories;
