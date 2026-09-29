//! The Tauri command surface: thin wrappers that turn frontend calls into engine / repository calls.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;
use uuid::Uuid;

use crate::chat::{images, ChatEngine, ChatEvent, ChatSession, EventSink};
use crate::db::Db;
use crate::error::{Error, Result};
use crate::models::{
    Character, CharacterNotebookEntry, ChatMessage, Conversation, ExpressionImage, GroupChat, Lorebook, Persona,
    PromptPreset, QuestLogEntry,
};
use crate::providers::gemini::GeminiRequestOptions;
use crate::providers::ollama::OllamaRequestOptions;
use crate::providers::{ApiKeyProvider, LlmProviderRegistry, SecretStore};
use crate::repo::{characters, conversations, group_chats, lorebooks, notebook, personas, prompt_presets, quest_log};
use crate::secrets::KeyringSecretStore;
use crate::{library, settings};

pub struct AppState {
    pub db: Db,
    pub engine: ChatEngine,
    pub providers: Arc<LlmProviderRegistry>,
    pub secrets: Arc<KeyringSecretStore>,
    pub images_dir: PathBuf,
    pub data_dir: PathBuf,
}

/// Forwards engine events to the frontend over a Tauri channel.
struct ChannelSink(Channel<ChatEvent>);

impl EventSink for ChannelSink {
    fn emit(&self, event: ChatEvent) {
        // The window may have closed mid-reply; there is nobody left to tell.
        let _ = self.0.send(event);
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    /// Whether the provider is set up with an API key ("apiKey") or just a local server ("local").
    pub auth: &'static str,
    pub has_key: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub providers: Vec<ProviderInfo>,
    pub data_dir: String,
}

fn provider_info(state: &AppState, id: &str) -> ProviderInfo {
    let local = id == "ollama";
    ProviderInfo {
        id: id.to_string(),
        name: crate::chat::reply::provider_display_name(id).to_string(),
        auth: if local { "local" } else { "apiKey" },
        has_key: local || state.secrets.try_get_api_key(id).is_some(),
    }
}

#[tauri::command]
pub fn bootstrap(state: State<'_, AppState>) -> Bootstrap {
    Bootstrap {
        providers: state.providers.provider_ids().iter().map(|id| provider_info(&state, id)).collect(),
        data_dir: state.data_dir.to_string_lossy().into_owned(),
    }
}

// ---- library data ----

#[tauri::command]
pub fn list_characters(state: State<'_, AppState>) -> Result<Vec<Character>> {
    state.db.with(characters::get_all)
}

#[tauri::command]
pub fn list_personas(state: State<'_, AppState>) -> Result<Vec<Persona>> {
    state.db.with(personas::get_all)
}

#[tauri::command]
pub fn list_conversations(state: State<'_, AppState>) -> Result<Vec<Conversation>> {
    state.db.with(conversations::get_all)
}

// ---- opening chats ----

#[tauri::command]
pub fn open_assistant(state: State<'_, AppState>, force_new: bool) -> Result<ChatSession> {
    state.engine.open_assistant(force_new)
}

#[tauri::command]
pub fn open_character(state: State<'_, AppState>, character_id: Uuid, force_new: bool) -> Result<ChatSession> {
    state.engine.open_character(character_id, force_new)
}

#[tauri::command]
pub fn resume_conversation(state: State<'_, AppState>, conversation_id: Uuid) -> Result<ChatSession> {
    state.engine.resume(conversation_id)
}

#[tauri::command]
pub fn new_chat(state: State<'_, AppState>, conversation_id: Uuid) -> Result<ChatSession> {
    state.engine.new_chat_like(conversation_id)
}

#[tauri::command]
pub fn delete_conversation(state: State<'_, AppState>, conversation_id: Uuid) -> Result<()> {
    state.engine.delete_conversation(conversation_id)
}

#[tauri::command]
pub fn open_group(state: State<'_, AppState>, group_id: Uuid, force_new: bool) -> Result<ChatSession> {
    state.engine.open_group(group_id, force_new)
}

#[tauri::command]
pub async fn pick_speaker(
    state: State<'_, AppState>,
    conversation_id: Uuid,
    character_id: Uuid,
    on_event: Channel<ChatEvent>,
) -> Result<()> {
    let engine = state.engine.clone();
    engine.pick_speaker(conversation_id, character_id, Arc::new(ChannelSink(on_event))).await
}

#[tauri::command]
pub fn open_last_roleplay(state: State<'_, AppState>) -> Result<Option<ChatSession>> {
    state.engine.open_last_roleplay()
}

/// The in-story state shown beside a roleplay: quests and per-character notebooks.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryState {
    pub quests: Vec<QuestLogEntry>,
    pub notebook: Vec<CharacterNotebookEntry>,
}

#[tauri::command]
pub fn story_state(state: State<'_, AppState>, conversation_id: Uuid) -> Result<StoryState> {
    state.db.with(|c| {
        Ok(StoryState {
            quests: quest_log::get_by_conversation(c, conversation_id)?,
            notebook: notebook::get_by_conversation(c, conversation_id)?,
        })
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub message: ChatMessage,
    pub conversation_title: String,
}

/// Full-text search over every chat message, newest match first.
#[tauri::command]
pub fn search_messages(state: State<'_, AppState>, query: String) -> Result<Vec<SearchHit>> {
    state.db.with(|c| {
        let mut hits = Vec::new();
        for message in conversations::search_messages(c, &query, 50)? {
            let title = conversations::get(c, message.conversation_id)?.map(|conv| conv.title).unwrap_or_default();
            hits.push(SearchHit { message, conversation_title: title });
        }
        Ok(hits)
    })
}

// ---- generation ----

#[tauri::command]
pub async fn send_message(
    state: State<'_, AppState>,
    conversation_id: Uuid,
    text: String,
    attachments: Vec<String>,
    on_event: Channel<ChatEvent>,
) -> Result<()> {
    let engine = state.engine.clone();
    engine.send_message(conversation_id, text, attachments, Arc::new(ChannelSink(on_event))).await
}

#[tauri::command]
pub async fn swipe_next(
    state: State<'_, AppState>,
    conversation_id: Uuid,
    message_id: Uuid,
    on_event: Channel<ChatEvent>,
) -> Result<()> {
    let engine = state.engine.clone();
    engine.swipe_next(conversation_id, message_id, Arc::new(ChannelSink(on_event))).await
}

#[tauri::command]
pub fn swipe_previous(state: State<'_, AppState>, message_id: Uuid) -> Result<ChatMessage> {
    state.engine.swipe_previous(message_id)
}

#[tauri::command]
pub fn stop_generation(state: State<'_, AppState>, conversation_id: Uuid) {
    state.engine.stop(conversation_id);
}

#[tauri::command]
pub fn edit_message(state: State<'_, AppState>, message_id: Uuid, text: String) -> Result<ChatMessage> {
    state.engine.edit_message(message_id, &text)
}

#[tauri::command]
pub fn delete_message(state: State<'_, AppState>, message_id: Uuid) -> Result<()> {
    state.engine.delete_message(message_id)
}

// ---- per-conversation settings ----

#[tauri::command]
pub fn set_conversation_model(
    state: State<'_, AppState>,
    conversation_id: Uuid,
    provider_id: String,
    model: String,
) -> Result<Conversation> {
    state.engine.set_model(conversation_id, &provider_id, &model)
}

#[tauri::command]
pub fn set_conversation_preset(state: State<'_, AppState>, conversation_id: Uuid, preset_id: Uuid) -> Result<Conversation> {
    state.engine.set_preset(conversation_id, preset_id)
}

#[tauri::command]
pub fn set_conversation_persona(
    state: State<'_, AppState>,
    conversation_id: Uuid,
    persona_id: Option<Uuid>,
) -> Result<Conversation> {
    state.engine.set_persona(conversation_id, persona_id)
}

#[tauri::command]
pub fn set_assistant_prompt(state: State<'_, AppState>, conversation_id: Uuid, text: String) -> Result<Conversation> {
    state.engine.set_assistant_prompt(conversation_id, &text)
}

#[tauri::command]
pub fn set_search_override(
    state: State<'_, AppState>,
    conversation_id: Uuid,
    enabled: Option<bool>,
) -> Result<Conversation> {
    state.engine.set_search_override(conversation_id, enabled)
}

// ---- models and keys ----

#[tauri::command]
pub fn list_models(state: State<'_, AppState>, provider_id: String) -> Result<Vec<String>> {
    state.db.with(|c| settings::cached_models(c, &provider_id))
}

/// Live model discovery, cached for next time. Errors are the provider's own user-readable message.
#[tauri::command]
pub async fn refresh_models(state: State<'_, AppState>, provider_id: String) -> Result<Vec<String>> {
    let provider = state.providers.get(&provider_id)?;
    let models = provider.list_models().await.map_err(|e| Error::Other(crate::chat::reply::user_message(&e)))?;
    state.db.with(|c| settings::cache_models(c, &provider_id, &models))?;
    Ok(models)
}

#[tauri::command]
pub fn set_api_key(state: State<'_, AppState>, provider_id: String, key: String) -> Result<ProviderInfo> {
    let key = key.trim();
    if key.is_empty() {
        state.secrets.delete_api_key(&provider_id)?;
    } else {
        state.secrets.save_api_key(&provider_id, key)?;
    }
    Ok(provider_info(&state, &provider_id))
}

#[tauri::command]
pub fn get_gemini_options(state: State<'_, AppState>) -> Result<GeminiRequestOptions> {
    state.db.with(settings::load_gemini_options)
}

#[tauri::command]
pub fn save_gemini_options(state: State<'_, AppState>, options: GeminiRequestOptions) -> Result<()> {
    state.db.with(|c| settings::save_gemini_options(c, &options))
}

#[tauri::command]
pub fn get_ollama_options(state: State<'_, AppState>) -> Result<OllamaRequestOptions> {
    state.db.with(settings::load_ollama_options)
}

#[tauri::command]
pub fn save_ollama_options(state: State<'_, AppState>, options: OllamaRequestOptions) -> Result<()> {
    state.db.with(|c| settings::save_ollama_options(c, &options))
}

// ---- images ----

/// Copies a picked image into the app's own storage so it survives the original moving.
#[tauri::command]
pub fn store_image(state: State<'_, AppState>, path: String) -> Result<String> {
    let stored = images::store_file(&state.images_dir, Path::new(&path))?;
    Ok(stored.to_string_lossy().into_owned())
}

// ---- library: characters ----

#[tauri::command]
pub fn save_character(state: State<'_, AppState>, character: Character) -> Result<Character> {
    state.db.with(|c| library::save_character(c, character))
}

#[tauri::command]
pub fn delete_character(state: State<'_, AppState>, character_id: Uuid) -> Result<()> {
    state.db.with(|c| characters::delete(c, character_id))
}

/// Imports a SillyTavern `.json` or `.png` card.
#[tauri::command]
pub fn import_character(state: State<'_, AppState>, path: String) -> Result<Character> {
    state.db.with(|c| library::import_character(c, &state.images_dir, Path::new(&path)))
}

/// Writes the character (as currently edited) to `path` as a character-card-v2 JSON file.
#[tauri::command]
pub fn export_character_card(state: State<'_, AppState>, character: Character, path: String) -> Result<()> {
    let json = state.db.with(|c| library::export_character_card(c, &character))?;
    std::fs::write(&path, json).map_err(|e| Error::Other(format!("Couldn't write {path}: {e}")))
}

/// Imports a sprite folder: one image per emotion, named after the emotion.
#[tauri::command]
pub fn import_expression_folder(state: State<'_, AppState>, folder: String) -> Result<Vec<ExpressionImage>> {
    library::import_expression_folder(&state.images_dir, Path::new(&folder))
}

/// Deletes a stored picture - only ever files inside the app's own images folder.
#[tauri::command]
pub fn delete_image(state: State<'_, AppState>, path: String) -> bool {
    images::delete_if_owned(&state.images_dir, Path::new(&path))
}

// ---- library: personas, lorebooks, presets, group chats ----

#[tauri::command]
pub fn save_persona(state: State<'_, AppState>, persona: Persona) -> Result<Persona> {
    state.db.with(|c| library::save_persona(c, persona))
}

#[tauri::command]
pub fn delete_persona(state: State<'_, AppState>, persona_id: Uuid) -> Result<()> {
    state.db.with(|c| personas::delete(c, persona_id))
}

#[tauri::command]
pub fn list_lorebooks(state: State<'_, AppState>) -> Result<Vec<Lorebook>> {
    state.db.with(lorebooks::get_all)
}

#[tauri::command]
pub fn save_lorebook(state: State<'_, AppState>, lorebook: Lorebook) -> Result<Lorebook> {
    state.db.with(|c| library::save_lorebook(c, lorebook))
}

#[tauri::command]
pub fn delete_lorebook(state: State<'_, AppState>, lorebook_id: Uuid) -> Result<()> {
    state.db.with(|c| lorebooks::delete(c, lorebook_id))
}

/// All presets, seeding the two built-in ones on first use.
#[tauri::command]
pub fn list_presets(state: State<'_, AppState>) -> Result<Vec<PromptPreset>> {
    state.db.with(crate::chat::presets::get_all_or_seed)
}

#[tauri::command]
pub fn save_preset(state: State<'_, AppState>, preset: PromptPreset) -> Result<PromptPreset> {
    state.db.with(|c| library::save_preset(c, preset))
}

#[tauri::command]
pub fn delete_preset(state: State<'_, AppState>, preset_id: Uuid) -> Result<()> {
    state.db.with(|c| prompt_presets::delete(c, preset_id))
}

#[tauri::command]
pub fn list_group_chats(state: State<'_, AppState>) -> Result<Vec<GroupChat>> {
    state.db.with(group_chats::get_all)
}

#[tauri::command]
pub fn save_group_chat(state: State<'_, AppState>, group: GroupChat) -> Result<GroupChat> {
    state.db.with(|c| library::save_group_chat(c, group))
}

#[tauri::command]
pub fn delete_group_chat(state: State<'_, AppState>, group_id: Uuid) -> Result<()> {
    state.db.with(|c| group_chats::delete(c, group_id))
}
