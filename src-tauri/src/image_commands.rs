//! Tauri commands for picture generation. Kept apart from `commands.rs` because a picture runs
//! alongside the chat (you can keep talking while it renders), so it has its own job tracking.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::chat::ImagePromptDraft;
use crate::commands::AppState;
use crate::error::{Error, Result};
use crate::chat::images;
use crate::imagegen::launcher::ComfyLauncher;
use crate::imagegen::prompts::{self, CharacterShot, ImageMode};
use crate::imagegen::{self, BackendOptions, ImageGenSettings, ImageRequest};
use crate::models::{Character, ChatMessage};

/// Pictures being rendered, by conversation, so Stop can reach them.
pub struct ImageJobs {
    client: reqwest::Client,
    active: Mutex<HashMap<Uuid, CancellationToken>>,
    comfy: ComfyLauncher,
}

impl Default for ImageJobs {
    fn default() -> Self {
        Self::new(&std::env::temp_dir())
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
    /// `data_dir` gets ComfyUI's console output when LocalTavern starts it.
    pub fn new(data_dir: &Path) -> Self {
        Self {
            client: imagegen::client(),
            active: Mutex::new(HashMap::new()),
            comfy: ComfyLauncher::new(data_dir.join("comfyui.log")),
        }
    }

    /// Stops the ComfyUI we started, so it doesn't keep holding the graphics card after the app closes.
    pub fn shutdown(&self) {
        self.comfy.stop();
    }

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

/// Saving a launcher starts ComfyUI right away rather than at the next app start.
#[tauri::command]
pub async fn save_image_settings(
    state: State<'_, AppState>,
    jobs: State<'_, ImageJobs>,
    settings: ImageGenSettings,
) -> Result<ImageGenSettings> {
    let saved = state.db.with(|c| imagegen::settings::save(c, settings))?;
    jobs.comfy.start(&jobs.client, &saved).await?;
    Ok(saved)
}

/// Checkpoints, samplers and schedulers from the server in `settings` (not necessarily saved yet,
/// so Settings can test an address before keeping it).
#[tauri::command]
pub async fn image_backend_options(jobs: State<'_, ImageJobs>, settings: ImageGenSettings) -> Result<BackendOptions> {
    let settings = settings.sanitized();
    jobs.comfy.ensure_ready(&jobs.client, &settings, &CancellationToken::new()).await?;
    imagegen::options(&jobs.client, &settings).await
}

/// Starts ComfyUI with the app when Settings name a launcher (see `imagegen::launcher`).
pub fn start_comfy_on_launch(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (state, jobs) = (app.state::<AppState>(), app.state::<ImageJobs>());
        let started = match state.db.with(imagegen::settings::load) {
            Ok(settings) => jobs.comfy.start(&jobs.client, &settings).await,
            Err(e) => Err(e),
        };
        if let Err(e) = started {
            eprintln!("Couldn't start ComfyUI: {e}");
        }
    });
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

/// The draft for a photo a character sent in the story. Visual identity (base look, or appearance,
/// or written description) and usual outfit go into the prompt; `description` is only the shot the
/// reply named (pose, place, lighting). Portrait size, as a phone photo.
pub fn photo_draft_for(settings: &ImageGenSettings, character: Option<&Character>, description: &str) -> ImagePromptDraft {
    let (prompt, negative_prompt) = match character {
        Some(character) => {
            let style = &character.picture_style;
            let (look, lock) = prompts::character_visual_identity(character);
            let prefix = prompts::merge_prompt_parts(&[&settings.prompt_prefix, &style.prompt_prefix]);
            (
                prompts::story_photo_prompt(&prefix, &look, &style.default_outfit, description, lock),
                prompts::merge_prompt_parts(&[&settings.negative_prompt, &style.negative_prompt]),
            )
        }
        None => (prompts::combine(&settings.prompt_prefix, "", description), settings.negative_prompt.clone()),
    };
    ImagePromptDraft {
        mode: ImageMode::Character,
        prompt,
        negative_prompt,
        width: settings.width,
        height: settings.height,
        character_id: character.map(|c| c.id),
        photo: Some(description.trim().to_string()),
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

/// Renders a (reviewed) draft and adds the picture to the chat - or, with `into_message`, puts it
/// into that picture message (a photo whose render had failed). `None` when stopped.
#[tauri::command]
pub async fn generate_image(
    state: State<'_, AppState>,
    jobs: State<'_, ImageJobs>,
    conversation_id: Uuid,
    draft: ImagePromptDraft,
    into_message: Option<Uuid>,
) -> Result<Option<ChatMessage>> {
    if draft.prompt.trim().is_empty() {
        return Err(Error::Other("The picture prompt is empty.".into()));
    }
    let settings = state.db.with(imagegen::settings::load)?;
    let request = render_request(&settings, &draft);
    let (token, _guard) = jobs.claim(conversation_id)?;
    jobs.comfy.ensure_ready(&jobs.client, &settings, &token).await?;
    make_room(&state, &jobs, &settings).await;
    let rendered = imagegen::generate(&jobs.client, &settings, &request, &token).await;
    give_back(&jobs, &settings).await;
    let Some(image) = rendered? else {
        return Ok(None);
    };
    match into_message {
        Some(id) => state.engine.fill_picture(id, &image).map(Some),
        None => state.engine.add_picture(conversation_id, &draft, &image).map(Some),
    }
}

/// Keeps a story photo that couldn't be drawn in the chat, to be drawn again from there.
#[tauri::command]
pub fn add_failed_photo(state: State<'_, AppState>, conversation_id: Uuid, draft: ImagePromptDraft) -> Result<ChatMessage> {
    state.engine.add_failed_photo(conversation_id, &draft)
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
    let prefix = prompts::merge_prompt_parts(&[&settings.prompt_prefix, &style.prompt_prefix]);
    Ok(ImageRequest {
        prompt: prompts::character_picture_prompt(&prefix, &style.base_look, &clothing, shot),
        negative_prompt: prompts::merge_prompt_parts(&[&settings.negative_prompt, &style.negative_prompt]),
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
    jobs.comfy.ensure_ready(&jobs.client, &settings, &token).await?;
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
            photo: None,
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
    fn photo_drafts_use_base_look_or_fall_back_to_the_written_description() {
        let settings = ImageGenSettings { prompt_prefix: "best quality".into(), ..Default::default() };
        let mut nova = nova();
        nova.description = "{{char}} is an African woman with dark skin. She hates mornings.".into();
        let draft = photo_draft_for(&settings, Some(&nova), "winking, blonde, cafe at night");
        assert_eq!(
            draft.prompt,
            "best quality, winking, cafe at night, 1girl, silver hair, navy flight jacket"
        );
        assert!(!draft.prompt.contains("blonde"), "shot appearance is scrubbed when base look is locked: {}", draft.prompt);
        assert!(!draft.prompt.contains("African") && !draft.prompt.contains("mornings"), "base look wins over prose: {}", draft.prompt);
        assert_eq!((draft.width, draft.height, draft.character_id), (832, 1216, Some(nova.id)));
        assert_eq!(draft.photo.as_deref(), Some("winking, blonde, cafe at night"), "the shot is kept for the story, apart from the card");

        nova.picture_style.base_look.clear();
        nova.picture_style.default_outfit.clear();
        let from_description = photo_draft_for(&settings, Some(&nova), "winking, cafe at night");
        assert_eq!(
            from_description.prompt,
            "best quality, Nova is an African woman with dark skin. She hates mornings., winking, cafe at night"
        );
        assert!(from_description.prompt.contains("African woman with dark skin"), "{}", from_description.prompt);
        assert_eq!(
            from_description.prompt.matches("African woman").count(),
            1,
            "prose is not doubled: {}",
            from_description.prompt
        );

        let nobody = photo_draft_for(&settings, None, "a sunset over the harbour");
        assert_eq!((nobody.prompt.as_str(), nobody.character_id), ("best quality, a sunset over the harbour", None));
    }

    #[test]
    fn photo_drafts_and_character_pictures_merge_the_card_prompts() {
        let settings = ImageGenSettings {
            prompt_prefix: "best quality".into(),
            negative_prompt: "blurry".into(),
            ..Default::default()
        };
        let mut nova = nova();
        nova.picture_style.prompt_prefix = "score_9".into();
        nova.picture_style.negative_prompt = "blonde, pale skin".into();

        let draft = photo_draft_for(&settings, Some(&nova), "winking, cafe");
        assert!(draft.prompt.starts_with("best quality, score_9,"), "{}", draft.prompt);
        assert_eq!(draft.negative_prompt, "blurry, blonde, pale skin");

        let happy = CharacterShot::Mood { emotion: "happy".into() };
        let request = character_request(&settings, &nova, &happy, None).unwrap();
        assert!(request.prompt.starts_with("best quality, score_9, 1girl, silver hair"), "{}", request.prompt);
        assert_eq!(request.negative_prompt, "blurry, blonde, pale skin");
    }
}
