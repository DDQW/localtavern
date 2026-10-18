//! Picture prompts and picture messages, against the scripted provider of the engine tests.

use super::*;
use crate::chat::ImagePromptDraft;
use crate::imagegen::prompts::ImageMode;
use crate::imagegen::{GeneratedImage, ImageGenSettings, PromptStyle};

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 1];

fn png() -> GeneratedImage {
    GeneratedImage { data: PNG.to_vec(), mime_type: "image/png".into() }
}

fn draft(prompt: &str, character_id: Option<Uuid>) -> ImagePromptDraft {
    ImagePromptDraft {
        mode: ImageMode::Scene,
        prompt: prompt.into(),
        negative_prompt: "blurry".into(),
        width: 1216,
        height: 832,
        character_id,
    }
}

fn settings() -> ImageGenSettings {
    ImageGenSettings { prompt_prefix: "best quality".into(), negative_prompt: "blurry".into(), ..Default::default() }
}

#[tokio::test]
async fn the_model_describes_the_character_and_the_draft_gets_prefixes_and_size() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Hello there."), finish("STOP")]),
        Script::Items(vec![chunk("Sure! Here you go:\n\"full body portrait, silver hair, blue coat\""), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    let nova = f.engine.resume(conv).unwrap().characters[0].clone();
    no_clock(&f, conv); // after resume, which gives a clock-less roleplay one again
    send(&f, conv, "hi").await.0.unwrap();
    f.db.with(|c| crate::imagegen::settings::set_character_prefix(c, nova.id, "<lora:nova:0.8>")).unwrap();

    let draft = f.engine.image_prompt(conv, ImageMode::Character, None, &settings()).await.unwrap().unwrap();
    assert_eq!(draft.prompt, "best quality, <lora:nova:0.8>, full body portrait, silver hair, blue coat");
    assert_eq!(draft.negative_prompt, "blurry");
    assert_eq!((draft.width, draft.height), (832, 1216));
    assert_eq!(draft.character_id, Some(nova.id));
    assert!(!f.engine.is_generating(conv));

    let request = &f.provider.requests()[1];
    let last = request.history.last().unwrap();
    assert_eq!(last.role, LlmRole::User, "the request is a trailing user turn after Nova's reply");
    assert!(last.text.starts_with("[System note: Pause the roleplay. Describe Nova as they look right now"), "{}", last.text);
    assert!(last.text.contains("comma-separated"), "{}", last.text);
    let system = request.system_instruction.clone().unwrap_or_default();
    assert!(!system.contains("Write Nova's next reply"), "the preset's reply prompt is left out: {system}");
    assert_eq!(request.enable_web_search, Some(false));
}

#[tokio::test]
async fn persona_and_scene_modes_name_the_right_subject_and_shape() {
    let f = fixture(vec![
        Script::Items(vec![chunk("tall, scarf"), finish("STOP")]),
        Script::Items(vec![chunk("A rainy street at night."), finish("STOP")]),
    ]);
    add_persona(&f.db, "Alex");
    let conv = ready_roleplay(&f);
    no_clock(&f, conv);
    f.db.with(|c| conversations::add_message(c, &ChatMessage::new(conv, MessageRole::User, "We walk outside."))).unwrap();

    let persona = f.engine.image_prompt(conv, ImageMode::Persona, None, &settings()).await.unwrap().unwrap();
    assert_eq!(persona.character_id, None);
    let asked = f.provider.requests()[0].history.last().unwrap().text.clone();
    assert!(asked.starts_with("We walk outside.\n\n[System note:"), "folded into the user's turn: {asked}");
    assert!(asked.contains("Alex (the user's character)"), "{asked}");

    let natural = ImageGenSettings { prompt_style: PromptStyle::Natural, prompt_prefix: String::new(), ..settings() };
    let scene = f.engine.image_prompt(conv, ImageMode::Scene, None, &natural).await.unwrap().unwrap();
    assert_eq!(scene.prompt, "A rainy street at night.");
    assert_eq!((scene.width, scene.height), (1216, 832), "scenes are landscape");
    let asked = f.provider.requests()[1].history.last().unwrap().text.clone();
    assert!(asked.contains("two to four plain sentences"), "{asked}");
}

#[tokio::test]
async fn in_a_group_the_subject_is_the_chosen_character_or_the_last_speaker() {
    let f = fixture(vec![
        Script::Items(vec![chunk("close-up portrait, freckles"), finish("STOP")]),
        Script::Items(vec![chunk("close-up portrait, beard"), finish("STOP")]),
    ]);
    let alice = add_character(&f.db, "Alice", None, &[]);
    let bob = add_character(&f.db, "Bob", None, &[]);
    let group = add_group(&f, GroupActivationMode::Manual, &[&alice, &bob]);
    let conv = ready_group(&f, &group);
    no_clock(&f, conv);
    let mut said = ChatMessage::new(conv, MessageRole::Character, "Hm.");
    said.speaker_character_id = Some(bob.id);
    f.db.with(|c| conversations::add_message(c, &said)).unwrap();

    let last_speaker = f.engine.image_prompt(conv, ImageMode::Face, None, &settings()).await.unwrap().unwrap();
    assert_eq!(last_speaker.character_id, Some(bob.id));
    assert_eq!((last_speaker.width, last_speaker.height), (1024, 1024));
    assert!(f.provider.requests()[0].history.last().unwrap().text.contains("Bob's face"));

    let chosen = f.engine.image_prompt(conv, ImageMode::Face, Some(alice.id), &settings()).await.unwrap().unwrap();
    assert_eq!(chosen.character_id, Some(alice.id));
    assert!(f.provider.requests()[1].history.last().unwrap().text.contains("Alice's face"));
}

#[tokio::test]
async fn character_modes_need_a_character_and_empty_answers_are_errors() {
    let f = fixture(vec![Script::Items(vec![chunk("  \n"), finish("STOP")])]);
    let assistant = ready_assistant(&f);
    let error = f.engine.image_prompt(assistant, ImageMode::Character, None, &settings()).await.unwrap_err();
    assert!(error.to_string().contains("no character"), "{error}");
    let error = f.engine.image_prompt(assistant, ImageMode::LastMessage, None, &settings()).await.unwrap_err();
    assert!(error.to_string().contains("no message"), "{error}");

    let conv = ready_roleplay(&f);
    let error = f.engine.image_prompt(conv, ImageMode::Character, None, &settings()).await.unwrap_err();
    assert!(error.to_string().contains("empty description"), "{error}");
}

#[tokio::test]
async fn pictures_show_in_the_chat_but_never_reach_the_model() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Hello."), finish("STOP")]),
        Script::Items(vec![chunk("Nice picture."), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    no_clock(&f, conv);
    send(&f, conv, "hi").await.0.unwrap();
    let nova = f.engine.resume(conv).unwrap().characters[0].id;

    let picture = f.engine.add_picture(conv, &draft("full body portrait", Some(nova)), &png()).unwrap();
    assert_eq!(picture.role, MessageRole::Image);
    assert_eq!(picture.text, "full body portrait");
    assert_eq!(picture.speaker_character_id, Some(nova));
    let kept: ImagePromptDraft = serde_json::from_str(picture.raw_prompt_text.as_deref().unwrap()).unwrap();
    assert_eq!(kept, draft("full body portrait", Some(nova)), "the whole draft is kept for drawing again");
    let path = std::path::PathBuf::from(&picture.attachments[0].image_path);
    assert_eq!(std::fs::read(&path).unwrap(), PNG);

    let session = f.engine.resume(conv).unwrap();
    assert_eq!(session.messages.last().unwrap().id, picture.id, "the chat view shows the picture");

    send(&f, conv, "look at this").await.0.unwrap();
    let request = &f.provider.requests()[1];
    assert!(request.history.iter().all(|m| m.attachments.is_none()), "the picture isn't uploaded");
    assert!(request.history.iter().all(|m| !m.text.contains("full body portrait")), "nor is its prompt");

    f.engine.delete_picture(picture.id).unwrap();
    assert!(!path.exists(), "the stored file goes with the message");
    assert!(messages(&f, conv).iter().all(|m| m.id != picture.id));
    let reply = messages(&f, conv).into_iter().find(|m| m.role == MessageRole::Character).unwrap();
    assert!(f.engine.delete_picture(reply.id).is_err(), "only pictures can be deleted this way");
}

#[tokio::test]
async fn a_picture_as_the_last_message_doesnt_change_what_continue_extends() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Once upon"), finish("STOP")]),
        Script::Items(vec![chunk(" a time."), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    no_clock(&f, conv);
    send(&f, conv, "tell me a story").await.0.unwrap();
    f.engine.add_picture(conv, &draft("scenery", None), &png()).unwrap();

    let sink = Arc::new(VecSink::default());
    f.engine.continue_message(conv, sink.clone()).await.unwrap();
    let story = messages(&f, conv).into_iter().find(|m| m.role == MessageRole::Character).unwrap();
    assert_eq!(story.text, "Once upon a time.");
}

#[tokio::test]
async fn the_request_stays_last_even_with_the_world_clocks_note() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Hello."), finish("STOP")]),
        Script::Items(vec![chunk("scenery, tavern"), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f); // keeps its in-story clock
    send(&f, conv, "hi").await.0.unwrap();

    f.engine.image_prompt(conv, ImageMode::Scene, None, &settings()).await.unwrap().unwrap();
    let history = &f.provider.requests()[1].history;
    let last = history.last().unwrap();
    assert_eq!(last.role, LlmRole::User);
    assert!(last.text.starts_with("[System note: Current in-story date/time"), "{}", last.text);
    assert!(last.text.ends_with("no quotes.]"), "the picture request comes last: {}", last.text);
    let mentions = history.iter().filter(|m| m.text.contains("Pause the roleplay")).count();
    assert_eq!(mentions, 1, "and only once");
    assert!(history.windows(2).all(|w| !(w[0].role == LlmRole::User && w[1].role == LlmRole::User && w[0].text.is_empty())));
}
