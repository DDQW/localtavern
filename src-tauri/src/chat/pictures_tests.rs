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
        photo: None,
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
    let mut styled = nova.clone();
    styled.picture_style.base_look = "<lora:nova:0.8>".into();
    f.db.with(|c| characters::upsert(c, &styled)).unwrap();

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
    assert_eq!(
        request.generation_config.reasoning_effort,
        Some(crate::models::ReasoningEffort::Low),
        "a tag list needs no long reasoning"
    );
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
async fn a_story_photo_that_failed_stays_in_the_chat_reaches_the_model_and_can_be_drawn_in_place() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Here you go."), finish("STOP")]),
        Script::Items(vec![chunk("Cute, right?"), finish("STOP")]),
        Script::Items(vec![chunk(" Told you."), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    no_clock(&f, conv);
    send(&f, conv, "send me a pic").await.0.unwrap();
    let nova = f.engine.resume(conv).unwrap().characters[0].id;

    assert!(f.engine.add_failed_photo(conv, &draft("a picture", None)).is_err(), "only story photos are kept unrendered");
    let photo = ImagePromptDraft { photo: Some("selfie of Nova at the beach".into()), ..draft("best quality, selfie", Some(nova)) };
    let failed = f.engine.add_failed_photo(conv, &photo).unwrap();
    assert_eq!(failed.role, MessageRole::Image);
    assert!(failed.attachments.is_empty());
    assert_eq!(f.engine.resume(conv).unwrap().messages.last().unwrap().id, failed.id, "shown in the chat");

    send(&f, conv, "wow").await.0.unwrap();
    let history = &f.provider.requests()[1].history;
    let note = history.iter().find(|m| m.text.contains("selfie of Nova")).expect("the photo is in the context");
    assert_eq!(note.role, LlmRole::User);
    assert_eq!(note.text, "[System note: A photo was shared in the story here. It showed: selfie of Nova at the beach]");
    assert!(history.iter().all(|m| !m.text.contains("best quality")), "the render prompt isn't");

    let drawn = f.engine.fill_picture(failed.id, &png()).unwrap();
    assert_eq!(drawn.id, failed.id);
    assert_eq!(std::fs::read(&drawn.attachments[0].image_path).unwrap(), PNG);
    let order: Vec<Uuid> = messages(&f, conv).iter().map(|m| m.id).collect();
    assert_eq!(order.iter().position(|id| *id == failed.id), Some(2), "it stays where the photo was sent");

    // The note doesn't end the exchange: Continue still extends the last reply.
    f.engine.continue_message(conv, Arc::new(VecSink::default())).await.unwrap();
    let last_reply = messages(&f, conv).into_iter().filter(|m| m.role == MessageRole::Character).last().unwrap();
    assert_eq!(last_reply.text, "Cute, right? Told you.");
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

fn with_gown(f: &Fixture) -> (Character, Uuid) {
    let mut nova = add_character(&f.db, "Nova", None, &["happy", "sad"]);
    nova.description = "A star-mapper.".into();
    let gown = crate::models::Outfit {
        id: Uuid::new_v4(),
        name: "Ball gown".into(),
        clothing: "red ball gown, long gloves".into(),
        expressions: vec![
            crate::models::ExpressionImage { emotion: "happy".into(), image_path: "gown-happy.png".into() },
            crate::models::ExpressionImage { emotion: "blushing".into(), image_path: "gown-blush.png".into() },
        ],
    };
    let id = gown.id;
    nova.outfits.push(gown);
    f.db.with(|c| characters::upsert(c, &nova)).unwrap();
    (nova, id)
}

#[tokio::test]
async fn a_chosen_outfit_is_what_the_model_is_told_and_whose_moods_it_picks_from() {
    let f = fixture(vec![
        Script::Items(vec![chunk("Shall we dance? [emotion: blushing]"), finish("STOP")]),
    ]);
    let (nova, gown) = with_gown(&f);
    let conv = ready_character(&f, &nova);
    let mut world = f.engine.resume(conv).unwrap().conversation.world_state.unwrap();
    world.outfits.insert(nova.id, gown);
    f.engine.set_world_state(conv, world).unwrap();

    send(&f, conv, "You look lovely.").await.0.unwrap();
    let system = f.provider.requests()[0].system_instruction.clone().unwrap_or_default();
    assert!(system.contains("Currently wearing: red ball gown, long gloves"), "{system}");
    assert!(system.contains("happy, blushing"), "the emotion list is the gown's: {system}");

    let reply = messages(&f, conv).into_iter().find(|m| m.role == MessageRole::Character).unwrap();
    assert_eq!(reply.text, "Shall we dance?");
    assert_eq!(reply.detected_emotion.as_deref(), Some("blushing"), "a mood only the gown has is recognised");

    let session = f.engine.resume(conv).unwrap();
    assert_eq!(session.characters[0].expressions.len(), 2, "the chat view gets the character as stored");
    assert_eq!(session.characters[0].expressions[0].image_path, "happy.png");
    assert_eq!(session.characters[0].description, "A star-mapper.");
}

#[test]
fn dressing_falls_back_to_the_defaults() {
    let f = fixture(vec![]);
    let (nova, gown) = with_gown(&f);
    assert_eq!(nova.clone().dressed(None), nova, "no outfit chosen");
    assert_eq!(nova.clone().dressed(Some(Uuid::new_v4())), nova, "an outfit that was deleted");

    let mut bare = nova.clone();
    bare.outfits[0].expressions.clear();
    let dressed = bare.dressed(Some(gown));
    assert_eq!(dressed.expressions, nova.expressions, "an outfit without pictures keeps the default portraits");
    assert!(dressed.description.ends_with("Currently wearing: red ball gown, long gloves"));
}

#[tokio::test]
async fn a_photo_the_character_sends_is_announced_and_kept_out_of_the_text() {
    let f = fixture(vec![Script::Items(vec![
        chunk("She snaps a quick selfie and sends it.\n\n[photo: selfie of a young woman, silver hair, winking, neon cafe at night]"),
        finish("STOP"),
    ])]);
    let conv = ready_roleplay(&f);
    let nova = f.engine.resume(conv).unwrap().characters[0].id;

    let (result, events) = send(&f, conv, "Send me a pic?").await;
    result.unwrap();
    let system = f.provider.requests()[0].system_instruction.clone().unwrap_or_default();
    assert!(system.contains("Nova sends User photos whenever a real person would"), "{system}");
    assert!(system.contains("square brackets included"), "{system}");

    let reply = messages(&f, conv).into_iter().find(|m| m.role == MessageRole::Character).unwrap();
    assert_eq!(reply.text, "She snaps a quick selfie and sends it.");
    let sent: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::PhotoSent { message_id, character_id, description, .. } => Some((*message_id, *character_id, description.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(sent, [(reply.id, Some(nova), "selfie of a young woman, silver hair, winking, neon cafe at night".to_string())]);
}

#[tokio::test]
async fn no_photo_instruction_or_event_when_auto_photos_are_off() {
    let f = fixture(vec![Script::Items(vec![chunk("Here. [photo: a sunset]"), finish("STOP")])]);
    f.db.with(|c| crate::imagegen::settings::save(c, ImageGenSettings { auto_photos: false, ..Default::default() })).unwrap();
    let conv = ready_roleplay(&f);
    let (result, events) = send(&f, conv, "hi").await;
    result.unwrap();
    let system = f.provider.requests()[0].system_instruction.clone().unwrap_or_default();
    assert!(!system.contains("photos whenever") && !system.contains("[photo:"), "{system}");
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::PhotoSent { .. })), "nothing to draw");
    let reply = messages(&f, conv).into_iter().find(|m| m.role == MessageRole::Character).unwrap();
    assert_eq!(reply.text, "Here.", "a stray tag is still kept out of the story");
}

fn photos_sent(events: &[ChatEvent]) -> Vec<(Option<Uuid>, String)> {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::PhotoSent { character_id, description, .. } => Some((*character_id, description.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_photo_the_user_takes_is_described_by_the_reply_and_drawn() {
    let f = fixture(vec![
        Script::Items(vec![chunk("\"Nice shot!\"\n\n[user-photo: the harbour at dusk, gulls over the water]"), finish("STOP")]),
        Script::Items(vec![chunk("Nova laughs.\n[user-photo: Nova laughing on the pier, wind in her hair]"), finish("STOP")]),
        Script::Items(vec![chunk("\"Lovely evening.\""), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    let nova = f.engine.resume(conv).unwrap().characters[0].id;

    // A photo of something else: drawn without anyone's look.
    let (_, events) = send(&f, conv, "*Soso decides to make a picture of the harbour*").await;
    let system = f.provider.requests()[0].system_instruction.clone().unwrap();
    assert!(system.contains("If User takes a photo or picture in User's latest message, also describe that photo"), "{system}");
    assert_eq!(photos_sent(&events), [(None, "the harbour at dusk, gulls over the water".to_string())]);
    assert_eq!(messages(&f, conv)[1].text, "\"Nice shot!\"");

    // A photo of the character: drawn with her look.
    let (_, events) = send(&f, conv, "*snaps a photo of Nova*").await;
    assert_eq!(photos_sent(&events), [(Some(nova), "Nova laughing on the pier, wind in her hair".to_string())]);

    // No photo words, no extra instruction.
    send(&f, conv, "What a view.").await.0.unwrap();
    assert!(!f.provider.requests()[2].system_instruction.clone().unwrap().contains("[user-photo:"));
}

#[tokio::test]
async fn the_photo_texts_and_switches_from_settings_are_what_the_model_gets() {
    use crate::prompting::instructions::{self, Instructions};
    let f = fixture(vec![
        Script::Items(vec![chunk("Here.\n[user-photo: the harbour]"), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    let mut ins = Instructions { user_photos: false, intimate_photos: false, ..Default::default() };
    ins.texts.insert(instructions::PHOTO.into(), "{char} may text {user} a picture: [photo: <what it shows>]".into());
    f.db.with(|c| instructions::save(c, ins)).unwrap();

    let (_, events) = send(&f, conv, "*takes a picture of the harbour*").await;
    let system = f.provider.requests()[0].system_instruction.clone().unwrap();
    assert!(system.contains("Nova may text User a picture: [photo: <what it shows>]"), "{system}");
    assert!(!system.contains("photos whenever a real person would"), "the built-in text is replaced: {system}");
    assert!(!system.contains("nude") && !system.contains("[user-photo:"), "both switches are off: {system}");
    assert!(photos_sent(&events).is_empty(), "the user's photo isn't drawn while that's off");
    assert_eq!(messages(&f, conv)[1].text, "Here.", "its tag is still kept out of the story");
}

#[tokio::test]
async fn intimate_photos_are_offered_only_for_characters_who_arent_minors() {
    use chrono::Datelike;
    let f = fixture(vec![
        Script::Items(vec![chunk("hi"), finish("STOP")]),
        Script::Items(vec![chunk("hi"), finish("STOP")]),
    ]);
    let conv = ready_roleplay(&f);
    let mut nova = f.engine.resume(conv).unwrap().characters[0].clone();
    let story_date = world(&f, conv).unwrap().fictional_date_time.date();

    nova.birth_date = story_date.with_year(story_date.year() - 25);
    f.db.with(|c| characters::upsert(c, &nova)).unwrap();
    send(&f, conv, "hey").await.0.unwrap();
    assert!(f.provider.requests()[0].system_instruction.clone().unwrap().contains("suggestive or nude photos of Nova"));

    nova.birth_date = story_date.with_year(story_date.year() - 16);
    f.db.with(|c| characters::upsert(c, &nova)).unwrap();
    send(&f, conv, "hey").await.0.unwrap();
    let system = f.provider.requests()[1].system_instruction.clone().unwrap();
    assert!(!system.contains("nude") && !system.contains("intimate"), "never for a minor: {system}");
    assert!(system.contains("Nova sends User photos whenever a real person would"), "ordinary photos still: {system}");
}
