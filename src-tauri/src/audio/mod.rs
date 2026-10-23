//! Sound effects and voices from ElevenLabs. A reply can carry clips (`[sound: ...]`, `[voice: ...]`);
//! nothing is made - and no credits are spent - until the user presses a clip's play button.

pub mod settings;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{Error, Result};
use crate::providers::ApiKeyProvider;

/// The id the ElevenLabs API key is kept under in the credential store.
pub const PROVIDER_ID: &str = "elevenlabs";

const BASE_URL: &str = "https://api.elevenlabs.io";

/// One of the account's voices, for Settings to pick from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    pub id: String,
    pub name: String,
    /// "premade", "cloned", "generated" and the like.
    pub category: String,
}

/// Makes the audio. The app uses [`ElevenLabs`]; tests use a fake.
#[async_trait]
pub trait AudioMaker: Send + Sync {
    /// Whether an API key is saved.
    fn has_key(&self) -> bool;
    /// A sound effect for `prompt`, as MP3. `seconds` of `None` lets the service pick the length.
    async fn sound(&self, prompt: &str, seconds: Option<u32>) -> Result<Vec<u8>>;
    /// `text` spoken by `voice_id`, as MP3.
    async fn speech(&self, voice_id: &str, model: &str, text: &str) -> Result<Vec<u8>>;
    async fn voices(&self) -> Result<Vec<Voice>>;
}

pub struct ElevenLabs {
    client: reqwest::Client,
    keys: Arc<dyn ApiKeyProvider>,
    base_url: String,
}

impl ElevenLabs {
    pub fn new(client: reqwest::Client, keys: Arc<dyn ApiKeyProvider>) -> Self {
        Self { client, keys, base_url: BASE_URL.into() }
    }

    /// Talks to another server (tests).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn key(&self) -> Result<String> {
        self.keys
            .try_get_api_key(PROVIDER_ID)
            .ok_or_else(|| Error::Other("Add your ElevenLabs API key in Settings first.".into()))
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let response = request
            .header("xi-api-key", self.key()?)
            .send()
            .await
            .map_err(|e| Error::Other(format!("Couldn't reach ElevenLabs: {e}")))?;
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(Error::Other(failure_message(status.as_u16(), &body)))
    }

    async fn post(&self, path: &str, body: serde_json::Value) -> Result<Vec<u8>> {
        let request = self
            .client
            .post(format!("{}{path}", self.base_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&body)?);
        let bytes = self
            .send(request)
            .await?
            .bytes()
            .await
            .map_err(|e| Error::Other(format!("ElevenLabs cut the audio short: {e}")))?;
        if bytes.is_empty() {
            return Err(Error::Other("ElevenLabs sent back no audio.".into()));
        }
        Ok(bytes.to_vec())
    }
}

/// What ElevenLabs said went wrong, in a sentence: its own message when it gave one.
fn failure_message(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v.get("detail").cloned());
    let said = match &detail {
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        Some(detail) => detail.get("message").and_then(|m| m.as_str()).map(str::to_string),
        None => None,
    };
    match (status, said) {
        (401, _) => "ElevenLabs didn't accept the API key - check it in Settings.".into(),
        (_, Some(said)) => format!("ElevenLabs: {said}"),
        (status, None) => format!("ElevenLabs answered with an error ({status})."),
    }
}

/// A voice id goes into a URL path, so only the characters ElevenLabs uses are let through.
fn valid_voice_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[async_trait]
impl AudioMaker for ElevenLabs {
    fn has_key(&self) -> bool {
        self.keys.try_get_api_key(PROVIDER_ID).is_some()
    }

    async fn sound(&self, prompt: &str, seconds: Option<u32>) -> Result<Vec<u8>> {
        let mut body = json!({ "text": prompt });
        if let Some(seconds) = seconds.filter(|s| *s > 0) {
            body["duration_seconds"] = json!(seconds);
        }
        self.post("/v1/sound-generation", body).await
    }

    async fn speech(&self, voice_id: &str, model: &str, text: &str) -> Result<Vec<u8>> {
        if !valid_voice_id(voice_id) {
            return Err(Error::Other("That isn't an ElevenLabs voice id - pick a voice in Settings.".into()));
        }
        self.post(&format!("/v1/text-to-speech/{voice_id}"), json!({ "text": text, "model_id": model })).await
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        #[derive(Deserialize)]
        struct Listing {
            voices: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            voice_id: String,
            name: String,
            #[serde(default)]
            category: Option<String>,
        }
        let response = self.send(self.client.get(format!("{}/v1/voices", self.base_url))).await?;
        let text = response.text().await.map_err(|e| Error::Other(format!("Couldn't read ElevenLabs' answer: {e}")))?;
        let listing: Listing =
            serde_json::from_str(&text).map_err(|_| Error::Other("ElevenLabs sent an answer this app can't read.".into()))?;
        Ok(listing
            .voices
            .into_iter()
            .map(|v| Voice { id: v.voice_id, name: v.name, category: v.category.unwrap_or_default() })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::providers::keys::StaticApiKey;

    fn service(server: &MockServer, key: Option<&str>) -> ElevenLabs {
        let keys = StaticApiKey(key.map(str::to_string));
        ElevenLabs::new(reqwest::Client::new(), Arc::new(keys)).with_base_url(server.uri())
    }

    #[tokio::test]
    async fn a_sound_effect_is_asked_for_by_prompt_and_optional_length() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/sound-generation"))
            .and(header("xi-api-key", "k"))
            .and(body_json(json!({ "text": "door slams", "duration_seconds": 4 })))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"MP3".to_vec()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/sound-generation"))
            .and(body_json(json!({ "text": "rain" })))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"RAIN".to_vec()))
            .mount(&server)
            .await;

        let eleven = service(&server, Some("k"));
        assert_eq!(eleven.sound("door slams", Some(4)).await.unwrap(), b"MP3");
        assert_eq!(eleven.sound("rain", None).await.unwrap(), b"RAIN", "no length: the service picks");
        assert_eq!(eleven.sound("rain", Some(0)).await.unwrap(), b"RAIN", "zero means the same");
    }

    #[tokio::test]
    async fn speech_goes_to_the_voice_with_the_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/voice_1"))
            .and(body_json(json!({ "text": "Hello there", "model_id": "eleven_multilingual_v2" })))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"VOICE".to_vec()))
            .mount(&server)
            .await;

        let eleven = service(&server, Some("k"));
        assert_eq!(eleven.speech("voice_1", "eleven_multilingual_v2", "Hello there").await.unwrap(), b"VOICE");
        let bad = eleven.speech("../etc", "m", "x").await.unwrap_err().to_string();
        assert!(bad.contains("isn't an ElevenLabs voice id"), "{bad}");
    }

    #[tokio::test]
    async fn the_accounts_voices_are_listed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/voices"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"voices":[{"voice_id":"a1","name":"Rachel","category":"premade","labels":{}},{"voice_id":"b2","name":"Mine"}]}"#,
            ))
            .mount(&server)
            .await;
        let voices = service(&server, Some("k")).voices().await.unwrap();
        assert_eq!(
            voices,
            [
                Voice { id: "a1".into(), name: "Rachel".into(), category: "premade".into() },
                Voice { id: "b2".into(), name: "Mine".into(), category: String::new() },
            ]
        );
    }

    #[tokio::test]
    async fn failures_say_what_went_wrong() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/sound-generation"))
            .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"detail":{"status":"invalid_api_key","message":"Invalid API key"}}"#))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/v1"))
            .respond_with(
                ResponseTemplate::new(402).set_body_string(r#"{"detail":{"status":"quota_exceeded","message":"You have 0 credits left."}}"#),
            )
            .mount(&server)
            .await;

        let eleven = service(&server, Some("k"));
        assert!(eleven.sound("x", None).await.unwrap_err().to_string().contains("didn't accept the API key"));
        assert_eq!(eleven.speech("v1", "m", "x").await.unwrap_err().to_string(), "ElevenLabs: You have 0 credits left.");
        let none = service(&server, None);
        assert!(!none.has_key());
        assert!(none.sound("x", None).await.unwrap_err().to_string().contains("Add your ElevenLabs API key"));
        assert_eq!(failure_message(500, "oops"), "ElevenLabs answered with an error (500).");
        assert_eq!(failure_message(422, r#"{"detail":"Text is too long"}"#), "ElevenLabs: Text is too long");
    }
}
