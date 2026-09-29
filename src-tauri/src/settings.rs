//! App settings stored in the `AppSettings` table, using the same keys and JSON shapes as the C#
//! app (PascalCase field names, enum values by name) so both read each other's settings.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Result;
use crate::providers::gemini::{
    GeminiOptionsSource, GeminiRequestOptions, GeminiSafetyCategory, GeminiSafetyThreshold,
};
use crate::providers::ollama::{OllamaOptionsSource, OllamaRequestOptions};
use crate::repo::app_settings;

const GEMINI_KEY: &str = "gemini.requestOptions";
const OLLAMA_KEY: &str = "ollama.requestOptions";
const LAST_PROVIDER_KEY: &str = "app.provider.lastSelected";

// ---- Gemini ----

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredGemini {
    enable_google_search: bool,
    /// String keys and values: they survive schema/enum-ordinal changes better than numbers.
    safety_thresholds: HashMap<String, String>,
}

fn category_name(c: GeminiSafetyCategory) -> &'static str {
    match c {
        GeminiSafetyCategory::Harassment => "Harassment",
        GeminiSafetyCategory::HateSpeech => "HateSpeech",
        GeminiSafetyCategory::SexuallyExplicit => "SexuallyExplicit",
        GeminiSafetyCategory::DangerousContent => "DangerousContent",
    }
}

fn threshold_name(t: GeminiSafetyThreshold) -> &'static str {
    match t {
        GeminiSafetyThreshold::Unspecified => "Unspecified",
        GeminiSafetyThreshold::Off => "Off",
        GeminiSafetyThreshold::BlockNone => "BlockNone",
        GeminiSafetyThreshold::BlockOnlyHigh => "BlockOnlyHigh",
        GeminiSafetyThreshold::BlockMediumAndAbove => "BlockMediumAndAbove",
        GeminiSafetyThreshold::BlockLowAndAbove => "BlockLowAndAbove",
    }
}

fn parse_threshold(name: &str) -> Option<GeminiSafetyThreshold> {
    [
        GeminiSafetyThreshold::Unspecified,
        GeminiSafetyThreshold::Off,
        GeminiSafetyThreshold::BlockNone,
        GeminiSafetyThreshold::BlockOnlyHigh,
        GeminiSafetyThreshold::BlockMediumAndAbove,
        GeminiSafetyThreshold::BlockLowAndAbove,
    ]
    .into_iter()
    .find(|t| threshold_name(*t).eq_ignore_ascii_case(name))
}

pub fn load_gemini_options(conn: &Connection) -> Result<GeminiRequestOptions> {
    let mut options = GeminiRequestOptions::default();
    let Some(json) = app_settings::get(conn, GEMINI_KEY)?.filter(|j| !j.is_empty()) else {
        return Ok(options);
    };
    // A value we can't read falls back to defaults rather than breaking every Gemini request.
    let Ok(stored) = serde_json::from_str::<StoredGemini>(&json) else {
        return Ok(options);
    };
    options.enable_google_search = stored.enable_google_search;
    let defaults = options.safety_thresholds.clone();
    options.safety_thresholds = GeminiSafetyCategory::ALL
        .iter()
        .map(|c| {
            let threshold = stored
                .safety_thresholds
                .get(category_name(*c))
                .and_then(|v| parse_threshold(v))
                .unwrap_or(defaults[c]);
            (*c, threshold)
        })
        .collect::<BTreeMap<_, _>>();
    Ok(options)
}

pub fn save_gemini_options(conn: &Connection, options: &GeminiRequestOptions) -> Result<()> {
    let stored = StoredGemini {
        enable_google_search: options.enable_google_search,
        safety_thresholds: options
            .safety_thresholds
            .iter()
            .map(|(c, t)| (category_name(*c).to_string(), threshold_name(*t).to_string()))
            .collect(),
    };
    app_settings::set(conn, GEMINI_KEY, &serde_json::to_string(&stored)?)
}

// ---- Ollama ----

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredOllama {
    base_url: String,
    context_size: Option<i32>,
}

pub fn load_ollama_options(conn: &Connection) -> Result<OllamaRequestOptions> {
    let default = OllamaRequestOptions::default();
    let Some(json) = app_settings::get(conn, OLLAMA_KEY)?.filter(|j| !j.is_empty()) else {
        return Ok(default);
    };
    match serde_json::from_str::<StoredOllama>(&json) {
        Ok(stored) if !stored.base_url.trim().is_empty() => Ok(OllamaRequestOptions {
            base_url: stored.base_url,
            context_size: stored.context_size.filter(|c| *c > 0).unwrap_or(default.context_size),
        }),
        _ => Ok(default),
    }
}

pub fn save_ollama_options(conn: &Connection, options: &OllamaRequestOptions) -> Result<()> {
    let stored = StoredOllama { base_url: options.base_url.clone(), context_size: Some(options.context_size) };
    app_settings::set(conn, OLLAMA_KEY, &serde_json::to_string(&stored)?)
}

/// Live option sources for the providers: read from the database on every request, so a change in
/// Settings applies immediately without restarting anything.
pub struct DbGeminiOptions(pub Db);
pub struct DbOllamaOptions(pub Db);

impl GeminiOptionsSource for DbGeminiOptions {
    fn current(&self) -> GeminiRequestOptions {
        self.0.with(load_gemini_options).unwrap_or_default()
    }
}

impl OllamaOptionsSource for DbOllamaOptions {
    fn current(&self) -> OllamaRequestOptions {
        self.0.with(load_ollama_options).unwrap_or_default()
    }
}

// ---- last-selected provider/model and the model cache ----

pub fn last_selected_provider(conn: &Connection) -> Result<Option<String>> {
    Ok(app_settings::get(conn, LAST_PROVIDER_KEY)?.filter(|p| !p.is_empty()))
}

pub fn set_last_selected_provider(conn: &Connection, provider_id: &str) -> Result<()> {
    if provider_id.is_empty() {
        return Ok(());
    }
    app_settings::set(conn, LAST_PROVIDER_KEY, provider_id)
}

pub fn last_selected_model(conn: &Connection, provider_id: &str) -> Result<Option<String>> {
    Ok(app_settings::get(conn, &format!("{provider_id}.model.lastSelected"))?.filter(|m| !m.is_empty()))
}

pub fn set_last_selected_model(conn: &Connection, provider_id: &str, model: &str) -> Result<()> {
    if model.is_empty() {
        return Ok(());
    }
    app_settings::set(conn, &format!("{provider_id}.model.lastSelected"), model)
}

/// The last successful live model listing for a provider (empty if it has never been refreshed).
pub fn cached_models(conn: &Connection, provider_id: &str) -> Result<Vec<String>> {
    let json = app_settings::get(conn, &format!("{provider_id}.models.cache"))?;
    Ok(json.and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok()).unwrap_or_default())
}

pub fn cache_models(conn: &Connection, provider_id: &str, models: &[String]) -> Result<()> {
    app_settings::set(conn, &format!("{provider_id}.models.cache"), &serde_json::to_string(models)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn gemini_options_default_when_nothing_is_stored() {
        let conn = open_in_memory().unwrap();
        assert_eq!(load_gemini_options(&conn).unwrap(), GeminiRequestOptions::default());
    }

    #[test]
    fn gemini_options_round_trip_and_use_the_csharp_shape() {
        let conn = open_in_memory().unwrap();
        let mut options = GeminiRequestOptions { enable_google_search: true, ..Default::default() };
        options.safety_thresholds.insert(GeminiSafetyCategory::HateSpeech, GeminiSafetyThreshold::BlockMediumAndAbove);
        save_gemini_options(&conn, &options).unwrap();

        assert_eq!(load_gemini_options(&conn).unwrap(), options);

        let raw = app_settings::get(&conn, GEMINI_KEY).unwrap().unwrap();
        assert!(raw.contains("\"EnableGoogleSearch\":true"), "{raw}");
        assert!(raw.contains("\"HateSpeech\":\"BlockMediumAndAbove\""), "{raw}");
    }

    #[test]
    fn reads_gemini_options_written_by_the_csharp_app() {
        let conn = open_in_memory().unwrap();
        app_settings::set(
            &conn,
            GEMINI_KEY,
            r#"{"EnableGoogleSearch":true,"SafetyThresholds":{"Harassment":"BlockNone","Bogus":"Off","DangerousContent":"nonsense"}}"#,
        )
        .unwrap();
        let options = load_gemini_options(&conn).unwrap();
        assert!(options.enable_google_search);
        assert_eq!(options.safety_thresholds[&GeminiSafetyCategory::Harassment], GeminiSafetyThreshold::BlockNone);
        // Unknown values and missing categories fall back to the default (Off).
        assert_eq!(options.safety_thresholds[&GeminiSafetyCategory::DangerousContent], GeminiSafetyThreshold::Off);
        assert_eq!(options.safety_thresholds[&GeminiSafetyCategory::HateSpeech], GeminiSafetyThreshold::Off);
    }

    #[test]
    fn corrupt_gemini_settings_fall_back_to_defaults() {
        let conn = open_in_memory().unwrap();
        app_settings::set(&conn, GEMINI_KEY, "{not json").unwrap();
        assert_eq!(load_gemini_options(&conn).unwrap(), GeminiRequestOptions::default());
    }

    #[test]
    fn ollama_options_round_trip_and_read_the_csharp_shape() {
        let conn = open_in_memory().unwrap();
        assert_eq!(load_ollama_options(&conn).unwrap(), OllamaRequestOptions::default());

        save_ollama_options(&conn, &OllamaRequestOptions { base_url: "http://box:1234".into(), context_size: 16384 }).unwrap();
        let loaded = load_ollama_options(&conn).unwrap();
        assert_eq!((loaded.base_url.as_str(), loaded.context_size), ("http://box:1234", 16384));
        assert!(app_settings::get(&conn, OLLAMA_KEY).unwrap().unwrap().contains("\"BaseUrl\":\"http://box:1234\""));

        // Older C# data without a context size, or with a blank URL.
        app_settings::set(&conn, OLLAMA_KEY, r#"{"BaseUrl":"http://old:1"}"#).unwrap();
        let old = load_ollama_options(&conn).unwrap();
        assert_eq!((old.base_url.as_str(), old.context_size), ("http://old:1", 8192));
        app_settings::set(&conn, OLLAMA_KEY, r#"{"BaseUrl":"  "}"#).unwrap();
        assert_eq!(load_ollama_options(&conn).unwrap(), OllamaRequestOptions::default());
    }

    #[test]
    fn last_selected_values_and_model_cache_use_the_shared_keys() {
        let conn = open_in_memory().unwrap();
        assert_eq!(last_selected_provider(&conn).unwrap(), None);
        set_last_selected_provider(&conn, "deepseek").unwrap();
        set_last_selected_model(&conn, "deepseek", "deepseek-chat").unwrap();
        set_last_selected_model(&conn, "deepseek", "").unwrap(); // ignored
        assert_eq!(last_selected_provider(&conn).unwrap().as_deref(), Some("deepseek"));
        assert_eq!(last_selected_model(&conn, "deepseek").unwrap().as_deref(), Some("deepseek-chat"));

        assert!(cached_models(&conn, "gemini").unwrap().is_empty());
        cache_models(&conn, "gemini", &["a".into(), "b".into()]).unwrap();
        assert_eq!(cached_models(&conn, "gemini").unwrap(), ["a", "b"]);
        assert_eq!(app_settings::get(&conn, "gemini.models.cache").unwrap().unwrap(), r#"["a","b"]"#);
    }
}
