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
use crate::imagegen::prompts::ImageMode;
use crate::imagegen::{self, BackendOptions, ImageGenSettings, ImageRequest};
use crate::models::ChatMessage;

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

#[tauri::command]
pub fn get_character_image_prefix(state: State<'_, AppState>, character_id: Uuid) -> Result<String> {
    state.db.with(|c| imagegen::settings::character_prefix(c, character_id))
}

#[tauri::command]
pub fn set_character_image_prefix(state: State<'_, AppState>, character_id: Uuid, prefix: String) -> Result<()> {
    state.db.with(|c| imagegen::settings::set_character_prefix(c, character_id, &prefix))
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
    let Some(image) = imagegen::generate(&jobs.client, &settings, &request, &token).await? else {
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
}
