//! Reads a real database written by the C# app through the Rust repositories, to prove the storage
//! formats line up. Ignored by default because it needs local data:
//!
//! ```text
//! cargo test compat -- --ignored --nocapture
//! ```
//!
//! Set `LOCALTAVERN_DB` to point at a different file; the default is the copy in the Tauri app-data
//! folder. The file is copied to a temp dir first, so the original is never modified.

use std::path::PathBuf;

use crate::models::ConversationMode;
use crate::repo::*;

fn source_db() -> PathBuf {
    if let Ok(p) = std::env::var("LOCALTAVERN_DB") {
        return PathBuf::from(p);
    }
    let local = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA not set");
    PathBuf::from(local).join("me.woelki.localtavern").join("localtavern.db")
}

#[test]
#[ignore = "needs a real C# LocalTavern database"]
fn reads_a_real_csharp_database() {
    let src = source_db();
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("copy.db");
    std::fs::copy(&src, &copy).unwrap_or_else(|e| panic!("cannot copy {}: {e}", src.display()));

    let conn = crate::db::open(&copy).unwrap();

    let characters = characters::get_all(&conn).unwrap();
    let personas = personas::get_all(&conn).unwrap();
    let lorebooks = lorebooks::get_all(&conn).unwrap();
    let groups = group_chats::get_all(&conn).unwrap();
    let presets = prompt_presets::get_all(&conn).unwrap();
    let conversations = conversations::get_all(&conn).unwrap();
    let memories = user_memories::get_all(&conn).unwrap();

    let mut messages = 0;
    let mut with_alternates = 0;
    let mut with_attachments = 0;
    let mut world_states = 0;
    for c in &conversations {
        for m in conversations::get_messages(&conn, c.id).unwrap() {
            messages += 1;
            with_alternates += usize::from(!m.alternates.is_empty());
            with_attachments += usize::from(!m.attachments.is_empty());
        }
        world_states += usize::from(c.world_state.is_some());
    }

    println!("characters: {}", characters.len());
    println!("personas: {}", personas.len());
    println!("lorebooks: {} ({} entries)", lorebooks.len(), lorebooks.iter().map(|l| l.entries.len()).sum::<usize>());
    println!("group chats: {}", groups.len());
    println!("presets: {} ({} blocks)", presets.len(), presets.iter().map(|p| p.blocks.len()).sum::<usize>());
    println!(
        "conversations: {} ({} roleplay), world states: {world_states}",
        conversations.len(),
        conversations.iter().filter(|c| c.mode == ConversationMode::Roleplay).count()
    );
    println!("messages: {messages} ({with_alternates} with alternates, {with_attachments} with attachments)");
    println!("user memories: {}", memories.len());

    // Round trip: re-saving a character must not corrupt it.
    if let Some(c) = characters.first() {
        characters::upsert(&conn, c).unwrap();
        let again = characters::get(&conn, c.id).unwrap().unwrap();
        assert_eq!(again.name, c.name);
        assert_eq!(again.first_messages, c.first_messages);
        assert_eq!(again.expressions, c.expressions);
    }
}
