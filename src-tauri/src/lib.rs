pub mod chat;
pub mod commands;
pub mod db;
pub mod error;
pub mod group_chats;
pub mod holidays;
pub mod image_commands;
pub mod imagegen;
pub mod import;
pub mod library;
pub mod llm;
pub mod models;
pub mod picked;
pub mod powershell;
pub mod prompting;
pub mod providers;
pub mod repo;
pub mod secrets;
pub mod settings;

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
use crate::providers::vastai::VastAiProvider;
use crate::providers::{http, LlmProviderRegistry};
use crate::secrets::KeyringSecretStore;
use crate::settings::{DbGeminiOptions, DbOllamaOptions, DbVastAiOptions};

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

    let vast = Arc::new(VastAiProvider::new(client.clone(), secrets.clone(), Arc::new(DbVastAiOptions(db.clone()))));
    let registry = Arc::new(LlmProviderRegistry::new(vec![
        Arc::new(GeminiProvider::new(client.clone(), secrets.clone(), Arc::new(DbGeminiOptions(db.clone())))),
        Arc::new(DeepSeekProvider::new(client.clone(), secrets.clone())),
        Arc::new(MistralProvider::new(client.clone(), secrets.clone())),
        Arc::new(XaiSubscriptionProvider::new(client.clone(), xai.clone())),
        Arc::new(ChatGptSubscriptionProvider::new(client.clone(), chatgpt.clone())),
        Arc::new(OllamaProvider::new(client.clone(), Arc::new(DbOllamaOptions(db.clone())))),
        vast.clone(),
    ]));
    let sessions = HashMap::from([
        (providers::xai::PROVIDER_ID.to_string(), xai),
        (providers::openai::PROVIDER_ID.to_string(), chatgpt),
    ]);

    let images_dir = data_dir.join("images");
    let engine = ChatEngine::new(db.clone(), registry.clone(), images_dir.clone());
    engine.set_holiday_source(Arc::new(holidays::NagerHolidays::new(client.clone())));
    Ok(AppState { db, engine, providers: registry, vast, secrets, sessions, sign_ins: Default::default(), images_dir, data_dir })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let data_dir = resolve_data_dir(app)?;
            app.manage(build_state(data_dir)?);
            app.manage(image_commands::ImageJobs::default());
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
            commands::list_story_cast,
            commands::scan_story_cast,
            commands::save_story_character,
            commands::delete_story_character,
            commands::get_track_story_cast,
            commands::set_track_story_cast,
            commands::holiday_countries,
            commands::holiday_overview,
            commands::draft_scenario,
            commands::draft_greetings,
            commands::draft_lore_entries,
            commands::draft_schedule,
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
            commands::get_vastai_options,
            commands::save_vastai_options,
            commands::get_default_personas,
            commands::save_default_personas,
            commands::store_image,
            commands::open_group,
            commands::pick_speaker,
            commands::continue_message,
            commands::impersonate,
            commands::context_status,
            commands::compact_conversation,
            commands::clear_conversation_summary,
            commands::save_character,
            commands::delete_character,
            commands::import_character,
            commands::export_character_card,
            commands::import_expression_folder,
            commands::delete_image,
            commands::save_persona,
            commands::delete_persona,
            commands::render_appearance,
            commands::polish_appearance,
            commands::list_lorebooks,
            commands::save_lorebook,
            commands::delete_lorebook,
            commands::list_presets,
            commands::save_preset,
            commands::delete_preset,
            commands::list_group_chats,
            commands::save_group_chat,
            commands::delete_group_chat,
            image_commands::get_image_settings,
            image_commands::save_image_settings,
            image_commands::image_backend_options,
            image_commands::describe_character_look,
            image_commands::draw_character_picture,
            image_commands::stop_character_picture,
            image_commands::image_prompt,
            image_commands::generate_image,
            image_commands::photo_draft,
            image_commands::stop_image,
            image_commands::delete_picture,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Don't leave a rented GPU running (and billing) because the app was closed mid-grace-period.
            if let tauri::RunEvent::Exit = event {
                let vast = app.state::<AppState>().vast.clone();
                tauri::async_runtime::block_on(vast.shutdown());
            }
        });
}
