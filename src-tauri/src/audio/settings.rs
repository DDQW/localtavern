//! Sound and voice settings, stored as one JSON value in `AppSettings`. The ElevenLabs key itself
//! lives in the credential store, like the providers' keys.

use std::collections::HashMap;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Result;
use crate::repo::app_settings;

const SETTINGS_KEY: &str = "audio.settings";

/// ElevenLabs' multilingual text-to-speech model.
pub const DEFAULT_SPEECH_MODEL: &str = "eleven_multilingual_v2";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioSettings {
    /// Let characters add a sound effect (`[sound: ...]`) to a reply, as a play button.
    pub sounds: bool,
    /// Let characters speak a line aloud (`[voice: ...]`) when that makes sense, as a play button.
    pub voices: bool,
    /// Who speaks when the character has no voice of their own below. Empty: nobody does.
    pub default_voice_id: String,
    /// Each character's own ElevenLabs voice.
    pub character_voices: HashMap<Uuid, String>,
    pub speech_model: String,
    /// How long a sound effect runs, in seconds; 0 lets ElevenLabs pick (a flat, cheaper price).
    pub sound_seconds: u32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            sounds: true,
            voices: true,
            default_voice_id: String::new(),
            character_voices: HashMap::new(),
            speech_model: DEFAULT_SPEECH_MODEL.into(),
            sound_seconds: 0,
        }
    }
}

impl AudioSettings {
    fn sanitized(mut self) -> Self {
        self.default_voice_id = self.default_voice_id.trim().to_string();
        self.character_voices.retain(|_, voice| !voice.trim().is_empty());
        for voice in self.character_voices.values_mut() {
            *voice = voice.trim().to_string();
        }
        self.speech_model = self.speech_model.trim().to_string();
        if self.speech_model.is_empty() {
            self.speech_model = DEFAULT_SPEECH_MODEL.into();
        }
        // ElevenLabs takes 0.5 to 22 seconds (some docs say 30); stay inside the smaller range.
        self.sound_seconds = self.sound_seconds.min(22);
        self
    }

    /// The voice `character` speaks with: their own, else the default; `None` when neither is set.
    pub fn voice_for(&self, character: Option<Uuid>) -> Option<&str> {
        character
            .and_then(|id| self.character_voices.get(&id))
            .map(String::as_str)
            .or(Some(self.default_voice_id.as_str()))
            .filter(|voice| !voice.is_empty())
    }
}

pub fn load(conn: &Connection) -> Result<AudioSettings> {
    let Some(json) = app_settings::get(conn, SETTINGS_KEY)?.filter(|j| !j.is_empty()) else {
        return Ok(AudioSettings::default());
    };
    // A value we can't read falls back to defaults rather than breaking every reply.
    Ok(serde_json::from_str::<AudioSettings>(&json).unwrap_or_default().sanitized())
}

pub fn save(conn: &Connection, settings: AudioSettings) -> Result<AudioSettings> {
    let settings = settings.sanitized();
    app_settings::set(conn, SETTINGS_KEY, &serde_json::to_string(&settings)?)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn defaults_when_nothing_or_garbage_is_stored() {
        let conn = open_in_memory().unwrap();
        assert_eq!(load(&conn).unwrap(), AudioSettings::default());
        app_settings::set(&conn, SETTINGS_KEY, "{nope").unwrap();
        assert_eq!(load(&conn).unwrap(), AudioSettings::default());
    }

    #[test]
    fn round_trips_and_cleans_up_what_it_keeps() {
        let conn = open_in_memory().unwrap();
        let nova = Uuid::new_v4();
        let settings = AudioSettings {
            default_voice_id: "  abc ".into(),
            character_voices: HashMap::from([(nova, " xyz ".to_string()), (Uuid::new_v4(), "  ".to_string())]),
            speech_model: " ".into(),
            sound_seconds: 99,
            ..Default::default()
        };
        let saved = save(&conn, settings).unwrap();
        assert_eq!(load(&conn).unwrap(), saved);
        assert_eq!((saved.default_voice_id.as_str(), saved.speech_model.as_str(), saved.sound_seconds), ("abc", DEFAULT_SPEECH_MODEL, 22));
        assert_eq!(saved.character_voices, HashMap::from([(nova, "xyz".to_string())]), "a blank voice is no voice");

        app_settings::set(&conn, SETTINGS_KEY, r#"{"voices":false}"#).unwrap();
        let partial = load(&conn).unwrap();
        assert!(!partial.voices && partial.sounds, "missing fields keep their defaults");
    }

    #[test]
    fn a_character_speaks_with_their_own_voice_else_the_default() {
        let nova = Uuid::new_v4();
        let mut settings = AudioSettings::default();
        assert_eq!(settings.voice_for(Some(nova)), None);
        settings.default_voice_id = "default".into();
        assert_eq!(settings.voice_for(Some(nova)), Some("default"));
        assert_eq!(settings.voice_for(None), Some("default"));
        settings.character_voices.insert(nova, "novas".into());
        assert_eq!(settings.voice_for(Some(nova)), Some("novas"));
        assert_eq!(settings.voice_for(Some(Uuid::new_v4())), Some("default"));
    }
}
