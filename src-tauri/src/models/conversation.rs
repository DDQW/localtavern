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
    /// Which outfit each character is wearing in this story (character id -> outfit id); a
    /// character not listed wears their default.
    #[serde(default)]
    pub outfits: std::collections::BTreeMap<Uuid, Uuid>,
    /// Largest in-story jump one reply may make on its own, in minutes (0 = no limit). A model that
    /// overshoots ("3 days" for a coffee) is clamped to this.
    #[serde(default = "default_max_skip_minutes")]
    pub max_skip_minutes: i32,
    /// Track an arousal / climax meter per character: after each reply the story is read for what
    /// raised or lowered them (off unless switched on in the Story panel).
    #[serde(default = "default_meters_enabled")]
    pub meters_enabled: bool,
    /// Character id -> meter.
    #[serde(default)]
    pub meters: std::collections::BTreeMap<Uuid, Meter>,
}

fn default_meters_enabled() -> bool {
    false
}

fn default_max_skip_minutes() -> i32 {
    6 * 60
}

/// How aroused a character is and how close to orgasm, both 0-100, plus how many times they have
/// climaxed in this story. The model reports them each reply; the app keeps them in range.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Meter {
    pub arousal: u8,
    pub climax: u8,
    pub orgasms: u32,
}

impl Meter {
    /// Most a meter may rise / fall in one reply, so a model that jumps straight to 100 (or back
    /// to 0) can't skip the build-up.
    const MAX_RISE: i32 = 60;
    const MAX_FALL: i32 = 40;

    fn step(current: u8, reported: u8) -> u8 {
        let current = current as i32;
        (reported.min(100) as i32).clamp(current - Self::MAX_FALL, current + Self::MAX_RISE).clamp(0, 100) as u8
    }

    /// Takes the model's reading. Returns whether the character climaxed: the reply says so, or the
    /// climax meter filled up. It is then counted and emptied, and arousal drops off. What the reply
    /// narrates wins over the numbers, so a climax in the text always counts.
    pub fn apply(&mut self, arousal: Option<u8>, climax: Option<u8>, orgasm: bool) -> bool {
        if let Some(a) = arousal {
            self.arousal = Self::step(self.arousal, a);
        }
        if let Some(c) = climax {
            self.climax = Self::step(self.climax, c);
        }
        if orgasm || climax.is_some_and(|c| c >= 100) || self.climax >= 100 {
            self.orgasms += 1;
            self.climax = 0;
            self.arousal = (self.arousal as u32 * 6 / 10) as u8;
            return true;
        }
        false
    }
}

/// The clock and weather as they were before a reply, plus what that reply did to the clock.
/// Stored on the message so regenerating or deleting it can rewind the story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockSnapshot {
    pub fictional_date_time: NaiveDateTime,
    pub weather: WeatherCondition,
    pub weather_notes: Option<String>,
    #[serde(default)]
    pub meters: std::collections::BTreeMap<Uuid, Meter>,
    /// How far the reply moved the clock.
    #[serde(default)]
    pub advanced_minutes: i64,
    /// The model gave no (readable) time-skip tag, so the flat per-message tick was used.
    #[serde(default)]
    pub estimated: bool,
    /// The model asked for more than the limit and was held to it (the original ask, in minutes).
    #[serde(default)]
    pub clamped_from: Option<i64>,
}

impl ClockSnapshot {
    pub fn of(state: &WorldState) -> Self {
        Self {
            fictional_date_time: state.fictional_date_time,
            weather: state.weather,
            weather_notes: state.weather_notes.clone(),
            meters: state.meters.clone(),
            advanced_minutes: 0,
            estimated: false,
            clamped_from: None,
        }
    }

    /// Puts the clock, weather and meters back as they were (leaves the rest of the setting alone).
    pub fn restore(&self, state: &mut WorldState) {
        state.fictional_date_time = self.fictional_date_time;
        state.weather = self.weather;
        state.weather_notes = self.weather_notes.clone();
        state.meters = self.meters.clone();
    }
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
            outfits: Default::default(),
            max_skip_minutes: default_max_skip_minutes(),
            meters_enabled: false,
            meters: Default::default(),
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
    /// Set for a private talk (breakout): the group conversation it belongs to. `character_id` is
    /// then the member the user is talking to.
    #[serde(default)]
    pub parent_conversation_id: Option<Uuid>,
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
            parent_conversation_id: None,
            created_at: now,
            updated_at: now,
        }
    }
}
