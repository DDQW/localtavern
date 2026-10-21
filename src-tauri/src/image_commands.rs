//! Tauri commands for picture generation. Kept apart from `commands.rs` because a picture runs
//! alongside the chat (you can keep talking while it renders), so it has its own job tracking.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::State;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::chat::ImagePromptDraft;
use crate::commands::AppState;
use crate::error::{Error, Result};
use crate::chat::images;
use crate::imagegen::prompts::{self, CharacterShot, ImageMode};
use crate::imagegen::{self, BackendOptions, ImageGenSettings, ImageRequest};
use crate::models::{Character, ChatMessage};

/// Pictures being rendered, by conversation, so Stop can reach them.
pub struct ImageJobs {
    client: reqwest::Client,
    active: Mutex<HashMap<Uuid, CancellationToken>>,
}

impl Default for ImageJobs {
    fn default() -> Self {
        Self { client: imagegen::client(), active: Mutex::new(HashMap::new()) }
    }
}

/// Removes the job when the render ends, however it ends.
struct JobGuard<'a> {
    jobs: &'a ImageJobs,
    conversation_id: Uuid,
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        self.jobs.active.lock().unwrap().remove(&self.conversation_id);
    }
}

impl ImageJobs {
    fn claim(&self, conversation_id: Uuid) -> Result<(CancellationToken, JobGuard<'_>)> {
        let mut active = self.active.lock().unwrap();
        if active.contains_key(&conversation_id) {
            return Err(Error::Other("A picture is already being drawn for this chat.".into()));
        }
        let token = CancellationToken::new();
        active.insert(conversation_id, token.clone());
        Ok((token, JobGuard { jobs: self, conversation_id }))
    }
}

/// Frees the graphics card for the image server when Settings ask for it (see `imagegen::ollama`).
async fn make_room(state: &AppState, jobs: &ImageJobs, settings: &ImageGenSettings) {
    if !settings.unload_ollama_first {
        return;
    }
    let Ok(ollama) = state.db.with(crate::settings::load_ollama_options) else {
        return;
    };
    let unloaded = imagegen::ollama::unload_all(&jobs.client, &ollama.base_url).await;
    if !unloaded.is_empty() {
        eprintln!("Unloaded {} from Ollama to make room for a picture", unloaded.join(", "));
    }
}

/// After a picture (finished, failed or stopped): hands the graphics card back to Ollama.
async fn give_back(jobs: &ImageJobs, settings: &ImageGenSettings) {
    if settings.unload_ollama_first {
        imagegen::release_vram(&jobs.client, settings).await;
    }
}

/// The render request for a reviewed draft, using the active backend's checkpoint and sampler.
pub fn render_request(settings: &ImageGenSettings, draft: &ImagePromptDraft) -> ImageRequest {
    let backend = settings.active();
    ImageRequest {
        prompt: draft.prompt.trim().to_string(),
        negative_prompt: draft.negative_prompt.trim().to_string(),
        width: draft.width,
        height: draft.height,
        steps: settings.steps,
        cfg_scale: settings.cfg_scale,
        seed: settings.seed,
        model: backend.model.clone(),
        sampler: backend.sampler.clone(),
        scheduler: backend.scheduler.clone(),
    }
}

#[tauri::command]
pub fn get_image_settings(state: State<'_, AppState>) -> Result<ImageGenSettings> {
    state.db.with(imagegen::settings::load)
}

#[tauri::command]
pub fn save_image_settings(state: State<'_, AppState>, settings: ImageGenSettings) -> Result<ImageGenSettings> {
    state.db.with(|c| imagegen::settings::save(c, settings))
}

/// Checkpoints, samplers and schedulers from the server in `settings` (not necessarily saved yet,
/// so Settings can test an address before keeping it).
#[tauri::command]
pub async fn image_backend_options(jobs: State<'_, ImageJobs>, settings: ImageGenSettings) -> Result<BackendOptions> {
    imagegen::options(&jobs.client, &settings.sanitized()).await
}

/// The model-written prompt for a picture, for review. `None` when stopped (Stop generation).
#[tauri::command]
pub async fn image_prompt(
    state: State<'_, AppState>,
    conversation_id: Uuid,
    mode: ImageMode,
    character_id: Option<Uuid>,
) -> Result<Option<ImagePromptDraft>> {
    let settings = state.db.with(imagegen::settings::load)?;
    let engine = state.engine.clone();
    engine.image_prompt(conversation_id, mode, character_id, &settings).await
}

/// The draft for a photo a character sent in the story: the global prefix, the character's base
/// look (so it looks like them) and the model's description, in the portrait size a phone photo has.
pub fn photo_draft_for(settings: &ImageGenSettings, character: Option<&Character>, description: &str) -> ImagePromptDraft {
    let look = character.map(|c| c.picture_style.base_look.as_str()).unwrap_or_default();
    ImagePromptDraft {
        mode: ImageMode::Character,
        prompt: prompts::combine(&settings.prompt_prefix, look, description),
        negative_prompt: settings.negative_prompt.clone(),
        width: settings.width,
        height: settings.height,
        character_id: character.map(|c| c.id),
    }
}

#[tauri::command]
pub fn photo_draft(state: State<'_, AppState>, character_id: Option<Uuid>, description: String) -> Result<ImagePromptDraft> {
    let (settings, character) = state.db.with(|c| {
        let character = match character_id {
            Some(id) => crate::repo::characters::get(c, id)?,
            None => None,
        };
        Ok((imagegen::settings::load(c)?, character))
    })?;
    Ok(photo_draft_for(&settings, character.as_ref(), &description))
}

/// Renders a (reviewed) draft and adds the picture to the chat. `None` when stopped.
#[tauri::command]
pub async fn generate_image(
    state: State<'_, AppState>,
    jobs: State<'_, ImageJobs>,
    conversation_id: Uuid,
    draft: ImagePromptDraft,
) -> Result<Option<ChatMessage>> {
    if draft.prompt.trim().is_empty() {
        return Err(Error::Other("The picture prompt is empty.".into()));
    }
    let settings = state.db.with(imagegen::settings::load)?;
    let request = render_request(&settings, &draft);
    let (token, _guard) = jobs.claim(conversation_id)?;
    make_room(&state, &jobs, &settings).await;
    let rendered = imagegen::generate(&jobs.client, &settings, &request, &token).await;
    give_back(&jobs, &settings).await;
    let Some(image) = rendered? else {
        return Ok(None);
    };
    state.engine.add_picture(conversation_id, &draft, &image).map(Some)
}

#[tauri::command]
pub fn stop_image(jobs: State<'_, ImageJobs>, conversation_id: Uuid) {
    if let Some(token) = jobs.active.lock().unwrap().get(&conversation_id) {
        token.cancel();
    }
}

#[tauri::command]
pub fn delete_picture(state: State<'_, AppState>, message_id: Uuid) -> Result<()> {
    state.engine.delete_picture(message_id)
}

// ---- the character editor's picture sets ----

/// A character's two shared prompt lines, written by the chat model from the description.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookDraft {
    pub base_look: String,
    pub default_outfit: String,
}

#[tauri::command]
pub async fn describe_character_look(state: State<'_, AppState>, character: Character) -> Result<LookDraft> {
    let settings = state.db.with(imagegen::settings::load)?;
    let engine = state.engine.clone();
    let (base_look, default_outfit) = engine.describe_look(&character, settings.prompt_style).await?;
    Ok(LookDraft { base_look, default_outfit })
}

/// A drawn picture for the editor (not saved to the character until the editor is saved).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterPicture {
    pub path: String,
    /// The seed it was drawn with: the editor keeps it so the rest of the set matches.
    pub seed: i64,
}

/// The render request for one picture of a character's set. `outfit_id` picks the clothing (the
/// default outfit otherwise); the character's seed is used, or a new one when it has none yet.
pub fn character_request(
    settings: &ImageGenSettings,
    character: &Character,
    shot: &CharacterShot,
    outfit_id: Option<Uuid>,
) -> Result<ImageRequest> {
    let style = &character.picture_style;
    if style.base_look.trim().is_empty() {
        return Err(Error::Other("Fill in the base look first (or let the AI write it from the description).".into()));
    }
    let clothing = match outfit_id {
        Some(id) => {
            let outfit = character
                .outfits
                .iter()
                .find(|o| o.id == id)
                .ok_or_else(|| Error::Other("That outfit no longer exists.".into()))?;
            if outfit.clothing.trim().is_empty() { outfit.name.clone() } else { outfit.clothing.clone() }
        }
        None => style.default_outfit.clone(),
    };
    let (width, height) = prompts::character_picture_size(shot, settings.width, settings.height);
    let backend = settings.active();
    Ok(ImageRequest {
        prompt: prompts::character_picture_prompt(&settings.prompt_prefix, &style.base_look, &clothing, shot),
        negative_prompt: settings.negative_prompt.trim().to_string(),
        width,
        height,
        steps: settings.steps,
        cfg_scale: settings.cfg_scale,
        seed: if style.seed >= 0 { style.seed } else { imagegen::random_seed() },
        model: backend.model.clone(),
        sampler: backend.sampler.clone(),
        scheduler: backend.scheduler.clone(),
    })
}

/// Draws one mood or action picture of the character as edited (saved or not). `None` when stopped.
#[tauri::command]
pub async fn draw_character_picture(
    state: State<'_, AppState>,
    jobs: State<'_, ImageJobs>,
    character: Character,
    shot: CharacterShot,
    outfit_id: Option<Uuid>,
) -> Result<Option<CharacterPicture>> {
    let settings = state.db.with(imagegen::settings::load)?;
    let request = character_request(&settings, &character, &shot, outfit_id)?;
    let (token, _guard) = jobs.claim(character.id)?;
    make_room(&state, &jobs, &settings).await;
    let rendered = imagegen::generate(&jobs.client, &settings, &request, &token).await;
    give_back(&jobs, &settings).await;
    let Some(image) = rendered? else {
        return Ok(None);
    };
    let path = images::store_bytes(&state.images_dir, &image.data, &image.mime_type)?;
    Ok(Some(CharacterPicture { path: path.to_string_lossy().into_owned(), seed: request.seed }))
}

#[tauri::command]
pub fn stop_character_picture(jobs: State<'_, ImageJobs>, character_id: Uuid) {
    if let Some(token) = jobs.active.lock().unwrap().get(&character_id) {
        token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imagegen::ImageBackend;

    #[test]
    fn render_requests_use_the_active_backends_choices() {
        let mut settings = ImageGenSettings { steps: 30, cfg_scale: 5.0, seed: 7, ..Default::default() };
        settings.a1111.model = "a1111-model".into();
        settings.comfy.model = "comfy-model".into();
        let draft = ImagePromptDraft {
            mode: ImageMode::Face,
            prompt: "  close-up portrait  ".into(),
            negative_prompt: "blurry".into(),
            width: 1024,
            height: 1024,
            character_id: None,
        };

        let a = render_request(&settings, &draft);
        assert_eq!((a.prompt.as_str(), a.model.as_str(), a.sampler.as_str()), ("close-up portrait", "a1111-model", "Euler a"));
        assert_eq!((a.width, a.height, a.steps, a.cfg_scale, a.seed), (1024, 1024, 30, 5.0, 7));

        settings.backend = ImageBackend::ComfyUi;
        let c = render_request(&settings, &draft);
        assert_eq!((c.model.as_str(), c.sampler.as_str(), c.scheduler.as_str()), ("comfy-model", "euler", "normal"));
    }

    #[test]
    fn one_picture_at_a_time_per_chat() {
        let jobs = ImageJobs::default();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let first = jobs.claim(a).unwrap();
        assert!(jobs.claim(a).is_err());
        assert!(jobs.claim(b).is_ok(), "other chats are independent");
        drop(first);
        assert!(jobs.claim(a).is_ok(), "the slot frees up when the render ends");
    }

    fn nova() -> Character {
        let mut nova = Character { name: "Nova".into(), ..Default::default() };
        nova.picture_style.base_look = "1girl, silver hair".into();
        nova.picture_style.default_outfit = "navy flight jacket".into();
        nova.outfits.push(crate::models::Outfit {
            id: Uuid::new_v4(),
            name: "Ball gown".into(),
            clothing: "red ball gown, gloves".into(),
            expressions: vec![],
        });
        nova
    }

    #[test]
    fn character_pictures_use_the_look_the_outfit_and_the_shared_seed() {
        let settings = ImageGenSettings { prompt_prefix: "best quality".into(), ..Default::default() };
        let mut nova = nova();
        let happy = CharacterShot::Mood { emotion: "happy".into() };

        let fresh = character_request(&settings, &nova, &happy, None).unwrap();
        assert!(
            fresh.prompt.starts_with("best quality, 1girl, silver hair, navy flight jacket, upper body portrait"),
            "{}",
            fresh.prompt
        );
        assert!(fresh.seed >= 0, "a character without a seed gets a new one");
        assert_eq!((fresh.width, fresh.height), (1024, 1024));

        nova.picture_style.seed = 1234;
        let gown = nova.outfits[0].id;
        let dressed = character_request(&settings, &nova, &happy, Some(gown)).unwrap();
        assert!(dressed.prompt.contains("red ball gown, gloves") && !dressed.prompt.contains("flight jacket"), "{}", dressed.prompt);
        assert_eq!(dressed.seed, 1234);

        assert!(character_request(&settings, &nova, &happy, Some(Uuid::new_v4())).is_err());
        nova.picture_style.base_look.clear();
        assert!(character_request(&settings, &nova, &happy, None).unwrap_err().to_string().contains("base look"));
    }

    #[test]
    fn photo_drafts_put_the_characters_look_before_the_description() {
        let settings = ImageGenSettings { prompt_prefix: "best quality".into(), ..Default::default() };
        let nova = nova();
        let draft = photo_draft_for(&settings, Some(&nova), "selfie, winking, cafe at night");
        assert_eq!(draft.prompt, "best quality, 1girl, silver hair, selfie, winking, cafe at night");
        assert_eq!((draft.width, draft.height, draft.character_id), (832, 1216, Some(nova.id)));
        let nobody = photo_draft_for(&settings, None, "a sunset over the harbour");
        assert_eq!((nobody.prompt.as_str(), nobody.character_id), ("best quality, a sunset over the harbour", None));
    }
}
