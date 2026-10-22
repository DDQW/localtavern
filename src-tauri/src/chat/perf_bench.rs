//! Timings for long chats, against a file-backed scratch database and the scripted provider.
//! Ignored by default; run with
//! `cargo test --release --lib perf_bench -- --ignored --nocapture --test-threads 1`.

use std::time::Instant;

use super::*;
use crate::chat::session;
use crate::models::{Lorebook, WorldInfoEntry};
use crate::repo::lorebooks;

fn reply_script(words: usize) -> Script {
    let mut items: Vec<_> = (0..words).map(|i| chunk(&format!("word{} ", i % 97))).collect();
    items.push(finish("STOP"));
    Script::Items(items)
}

fn file_fixture(dir: &std::path::Path, scripts: Vec<Script>) -> Fixture {
    let provider = FakeProvider::new(scripts);
    let db = Db::new(crate::db::open(&dir.join("bench.db")).unwrap());
    let images = tempfile::tempdir().unwrap();
    let registry = LlmProviderRegistry::new(vec![provider.clone()]);
    let engine = ChatEngine::new(db.clone(), Arc::new(registry), images.path().to_path_buf());
    engine.set_extraction(false);
    Fixture { engine, provider, db, _images: images }
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn long_roleplay() {
    const TURNS: usize = 300;
    const REPLY_WORDS: usize = 250;
    let dir = tempfile::tempdir().unwrap();
    let f = file_fixture(dir.path(), (0..TURNS).map(|_| reply_script(REPLY_WORDS)).collect());

    let book = Lorebook {
        name: "World".into(),
        entries: (0..150)
            .map(|i| WorldInfoEntry {
                keys: vec![format!("place{i}"), format!("word{}", i % 97)],
                content: format!("Lore entry {i}. {}", "Some detail about the world. ".repeat(10)),
                insertion_order: i,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    f.db.with(|c| lorebooks::upsert(c, &book)).unwrap();
    let nova = Character {
        name: "Nova".into(),
        description: "A starship captain with a dry wit. ".repeat(80),
        personality: "Calm, curious, stubborn. ".repeat(20),
        scenario: "The ship drifts near a nebula. ".repeat(10),
        linked_lorebook_ids: vec![book.id],
        ..Default::default()
    };
    f.db.with(|c| characters::upsert(c, &nova)).unwrap();
    add_persona(&f.db, "Alex");
    let conv = ready_character(&f, &nova);

    let mut turn_ms = Vec::new();
    for i in 0..TURNS {
        let start = Instant::now();
        let (result, _) = send(&f, conv, &format!("Turn {i}: I look around place{} and ask what she sees. {}", i % 150, "More words here. ".repeat(10))).await;
        result.unwrap();
        turn_ms.push(ms(start));
    }
    for at in [0, 49, 99, 199, TURNS - 1] {
        let window = &turn_ms[at.saturating_sub(4)..=at];
        println!("turn {:>4}: {:>8.2} ms (avg of last {})", at + 1, window.iter().sum::<f64>() / window.len() as f64, window.len());
    }

    let (raw, alternates, text): (i64, i64, i64) = f
        .db
        .with(|c| {
            Ok(c.query_row(
                "SELECT SUM(LENGTH(RawPromptText)), SUM(LENGTH(AlternatesJson)), SUM(LENGTH(Text)) FROM ChatMessages;",
                [],
                |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get(1)?, r.get(2)?)),
            )?)
        })
        .unwrap();
    let prompts: i64 = f
        .db
        .with(|c| Ok(c.query_row("SELECT COALESCE(SUM(LENGTH(Prompt)), 0) FROM MessagePrompts;", [], |r| r.get(0))?))
        .unwrap();
    println!("MessagePrompts {:.1} MB", prompts as f64 / 1e6);
    let size = std::fs::metadata(dir.path().join("bench.db")).unwrap().len();
    println!("db file {:.1} MB; RawPromptText {:.1} MB, AlternatesJson {:.1} MB, Text {:.2} MB",
        size as f64 / 1e6, raw as f64 / 1e6, alternates as f64 / 1e6, text as f64 / 1e6);

    let start = Instant::now();
    let ctx = f.db.with(|c| session::load_reply_context(c, conv)).unwrap();
    println!("load_reply_context: {:>8.2} ms ({} messages)", ms(start), ctx.messages.len());

    let start = Instant::now();
    let opened = f.engine.resume(conv).unwrap();
    println!("resume (build_session): {:>8.2} ms", ms(start));

    let start = Instant::now();
    let json = serde_json::to_string(&opened).unwrap();
    println!("session JSON: {:.1} MB in {:.2} ms", json.len() as f64 / 1e6, ms(start));

    let start = Instant::now();
    let found = f.db.with(|c| conversations::search_messages(c, "nebula word5", 50)).unwrap();
    println!("search: {:>8.2} ms ({} hits)", ms(start), found.len());

    let start = Instant::now();
    let msgs = messages(&f, conv);
    let last = msgs.last().unwrap();
    f.engine.edit_message(last.id, "edited").unwrap();
    println!("edit_message: {:>8.2} ms", ms(start));

    let start = Instant::now();
    let status = f.engine.context_status(conv).await.unwrap();
    println!("context_status: {:>8.2} ms ({status:?})", ms(start));
    let _ = f.provider.requests().len();
}

#[test]
#[ignore]
fn query_plans() {
    let conn = crate::db::open_in_memory().unwrap();
    for sql in [
        "EXPLAIN QUERY PLAN SELECT m.*, EXISTS (SELECT 1 FROM MessagePrompts p WHERE p.MessageId = m.Id) AS HasPrompt FROM ChatMessages m WHERE m.ConversationId = 'x' ORDER BY m.RowId;",
        "EXPLAIN QUERY PLAN INSERT OR REPLACE INTO MessagePrompts (MessageId, AlternateIndex, Prompt) VALUES ('a', 0, 'p');",
        "EXPLAIN QUERY PLAN UPDATE ChatMessages SET Text = 'x' WHERE Id = 'a';",
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        let rows: Vec<String> = stmt.query_map([], |r| r.get::<_, String>(3)).unwrap().map(|r| r.unwrap()).collect();
        println!("{sql}\n  {rows:?}");
    }
}
