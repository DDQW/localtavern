use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationMode {
    Roleplay,
    Assistant,
}
int_enum!(ConversationMode { Roleplay = 0, Assistant = 1 });

/// Append-only: persisted by name inside `WorldStateJson`, so members must never be renamed or removed.
/// Serialized as PascalCase to match the names the C# app wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeatherCondition {
    Clear,
    PartlyCloudy,
    Overcast,
    Rain,
    Thunderstorm,
    Snow,
    Fog,
    Windy,
}

/// Per-conversation in-story date/time and weather.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldState {
    pub fictional_date_time: NaiveDateTime,
    pub weather: WeatherCondition,
    pub weather_notes: Option<String>,
    /// Fallback tick only - the model normally estimates each reply's elapsed time itself.
    pub default_advance_minutes_per_message: i32,
    pub auto_drift_weather_enabled: bool,
}

impl WorldState {
    pub fn create_default() -> Self {
        Self {
            fictional_date_time: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().and_hms_opt(9, 0, 0).unwrap(),
            weather: WeatherCondition::Clear,
            weather_notes: None,
            default_advance_minutes_per_message: 5,
            auto_drift_weather_enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: Uuid,
    pub title: String,
    pub mode: ConversationMode,
    pub character_id: Option<Uuid>,
    pub group_chat_id: Option<Uuid>,
    pub persona_id: Option<Uuid>,
    pub prompt_preset_id: Option<Uuid>,
    pub provider_id: String,
    pub model: String,
    /// User-editable system prompt, only used in Assistant mode.
    pub assistant_system_prompt: Option<String>,
    /// Per-conversation override for search grounding; `None` means use the app-wide default.
    pub enable_google_search: Option<bool>,
    /// Only meaningful in Roleplay mode. `None` means not set up yet.
    pub world_state: Option<WorldState>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Default for Conversation {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: String::new(),
            mode: ConversationMode::Assistant,
            character_id: None,
            group_chat_id: None,
            persona_id: None,
            prompt_preset_id: None,
            provider_id: "gemini".into(),
            model: String::new(),
            assistant_system_prompt: None,
            enable_google_search: None,
            world_state: None,
            created_at: now,
            updated_at: now,
        }
    }
}
