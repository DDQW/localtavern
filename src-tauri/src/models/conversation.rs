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

impl WeatherCondition {
    pub const ALL: [Self; 8] = [
        Self::Clear,
        Self::PartlyCloudy,
        Self::Overcast,
        Self::Rain,
        Self::Thunderstorm,
        Self::Snow,
        Self::Fog,
        Self::Windy,
    ];

    /// The variant name, as shown in prompts ("PartlyCloudy") - matches the C# `ToString()`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Clear => "Clear",
            Self::PartlyCloudy => "PartlyCloudy",
            Self::Overcast => "Overcast",
            Self::Rain => "Rain",
            Self::Thunderstorm => "Thunderstorm",
            Self::Snow => "Snow",
            Self::Fog => "Fog",
            Self::Windy => "Windy",
        }
    }
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
    /// Where the story is set (ISO 3166-1 alpha-2, e.g. "DE"); picks the public holidays.
    #[serde(default)]
    pub country_code: Option<String>,
    /// Subdivision for regional holidays, as the holiday data names it (e.g. "DE-BY").
    #[serde(default)]
    pub region_code: Option<String>,
    /// Extra or fictional holidays for this story.
    #[serde(default)]
    pub custom_holidays: Vec<CustomHoliday>,
}

/// A holiday the user added to a story: every year on `month`/`day`, or once when `year` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomHoliday {
    pub name: String,
    pub month: u32,
    pub day: u32,
    #[serde(default)]
    pub year: Option<i32>,
}

impl CustomHoliday {
    pub fn falls_on(&self, date: NaiveDate) -> bool {
        use chrono::Datelike;
        date.month() == self.month && date.day() == self.day && self.year.is_none_or(|y| y == date.year())
    }
}

impl WorldState {
    pub fn create_default() -> Self {
        Self {
            fictional_date_time: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().and_hms_opt(9, 0, 0).unwrap(),
            weather: WeatherCondition::Clear,
            weather_notes: None,
            default_advance_minutes_per_message: 5,
            auto_drift_weather_enabled: false,
            country_code: None,
            region_code: None,
            custom_holidays: Vec::new(),
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
