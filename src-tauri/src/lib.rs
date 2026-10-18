pub mod chat;
pub mod commands;
pub mod db;
pub mod error;
pub mod group_chats;
pub mod import;
pub mod library;
pub mod llm;
pub mod models;
pub mod powershell;
pub mod prompting;
pub mod providers;
pub mod repo;
pub mod secrets;
pub mod settings;
pub mod story;

#[cfg(test)]
mod compat_tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;

use crate::chat::ChatEngine;
use crate::commands::AppState;
use crate::db::Db;
use crate::providers::deepseek::DeepSeekProvider;
use crate::providers::gemini::GeminiProvider;
use crate::providers::mistral::MistralProvider;
use crate::providers::oauth::{OAuthPkceClient, OAuthSession};
use crate::providers::openai::ChatGptSubscriptionProvider;
use crate::providers::xai::XaiSubscriptionProvider;
use crate::providers::ollama::OllamaProvider;
use crate::providers::{http, LlmProviderRegistry};
use crate::secrets::KeyringSecretStore;
use crate::settings::{DbGeminiOptions, DbOllamaOptions};

/// Where the database and images live: `LOCALTAVERN_DATA_DIR` if set (handy for trying the app on a
/// scratch copy), otherwise the OS per-user app-data folder for this app's identifier.
fn resolve_data_dir(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(dir) = std::env::var_os("LOCALTAVERN_DATA_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(app.path().app_local_data_dir()?)
}

fn build_state(data_dir: PathBuf) -> Result<AppState, Box<dyn std::error::Error>> {
    let db = Db::new(db::open(&data_dir.join("localtavern.db"))?);
    let secrets = Arc::new(KeyringSecretStore);
    let client = http::build_client();

    // Subscription logins: one persistent OAuth session per provider.
    let session = |config, key: &str| {
        Arc::new(OAuthSession::new(OAuthPkceClient::new(client.clone(), config), secrets.clone(), key))
    };
    let xai = session(providers::xai::oauth_config(), providers::xai::SECRET_KEY);
    let chatgpt = session(providers::openai::oauth_config(), providers::openai::SECRET_KEY);

    let registry = Arc::new(LlmProviderRegistry::new(vec![
        Arc::new(GeminiProvider::new(client.clone(), secrets.clone(), Arc::new(DbGeminiOptions(db.clone())))),
        Arc::new(DeepSeekProvider::new(client.clone(), secrets.clone())),
        Arc::new(MistralProvider::new(client.clone(), secrets.clone())),
        Arc::new(XaiSubscriptionProvider::new(client.clone(), xai.clone())),
        Arc::new(ChatGptSubscriptionProvider::new(client.clone(), chatgpt.clone())),
        Arc::new(OllamaProvider::new(client.clone(), Arc::new(DbOllamaOptions(db.clone())))),
    ]));
    let sessions = HashMap::from([
        (providers::xai::PROVIDER_ID.to_string(), xai),
        (providers::openai::PROVIDER_ID.to_string(), chatgpt),
    ]);

    let images_dir = data_dir.join("images");
    let engine = ChatEngine::new(db.clone(), registry.clone(), images_dir.clone());
    Ok(AppState { db, engine, providers: registry, secrets, sessions, sign_ins: Default::default(), images_dir, data_dir })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = resolve_data_dir(app)?;
            app.manage(build_state(data_dir)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::list_characters,
            commands::list_personas,
            commands::list_conversations,
            commands::open_assistant,
            commands::open_character,
            commands::resume_conversation,
            commands::open_last_roleplay,
            commands::story_state,
            commands::add_note,
            commands::update_note,
            commands::delete_note,
            commands::search_messages,
            commands::new_chat,
            commands::delete_conversation,
            commands::send_message,
            commands::swipe_next,
            commands::swipe_previous,
            commands::stop_generation,
            commands::edit_message,
            commands::delete_message,
            commands::set_conversation_model,
            commands::set_conversation_preset,
            commands::set_conversation_persona,
            commands::set_assistant_prompt,
            commands::set_search_override,
            commands::set_world_state,
            commands::list_models,
            commands::refresh_models,
            commands::set_api_key,
            commands::browser_key_sign_in,
            commands::get_powershell_options,
            commands::save_powershell_options,
            commands::resolve_tool_approval,
            commands::list_memories,
            commands::delete_memory,
            commands::clear_memories,
            commands::subscription_status,
            commands::subscription_sign_in,
            commands::subscription_cancel_sign_in,
            commands::subscription_sign_out,
            commands::get_gemini_options,
            commands::save_gemini_options,
            commands::get_ollama_options,
            commands::save_ollama_options,
            commands::get_default_personas,
            commands::save_default_personas,
            commands::store_image,
            commands::open_group,
            commands::pick_speaker,
            commands::continue_message,
            commands::impersonate,
            commands::save_character,
            commands::delete_character,
            commands::import_character,
            commands::export_character_card,
            commands::import_expression_folder,
            commands::delete_image,
            commands::save_persona,
            commands::delete_persona,
            commands::list_lorebooks,
            commands::save_lorebook,
            commands::delete_lorebook,
            commands::list_presets,
            commands::save_preset,
            commands::delete_preset,
            commands::list_group_chats,
            commands::save_group_chat,
            commands::delete_group_chat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
