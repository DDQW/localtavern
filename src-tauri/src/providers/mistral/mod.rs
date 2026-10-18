//! Mistral (`api.mistral.ai`), an OpenAI-compatible API authenticated with a bearer API key. The key
//! is either pasted in or obtained through Mistral's browser sign-in ([`signin`]), which hands back
//! an ordinary API key for the signed-in AI Studio account.

mod errors;
pub mod signin;
pub mod wire;

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;

use super::error::ProviderError;
use super::http::{ensure_success, send, transport_error};
use super::keys::ApiKeyProvider;
use super::{openai_compat, ChunkStream, LlmProvider};
use crate::llm::LlmRequest;

pub const PROVIDER_ID: &str = "mistral";
const DEFAULT_BASE_URL: &str = "https://api.mistral.ai/v1";

pub struct MistralProvider {
    client: reqwest::Client,
    api_keys: Arc<dyn ApiKeyProvider>,
    base_url: String,
}

impl MistralProvider {
    pub fn new(client: reqwest::Client, api_keys: Arc<dyn ApiKeyProvider>) -> Self {
        Self { client, api_keys, base_url: DEFAULT_BASE_URL.to_string() }
    }

    /// Points the provider at another server (tests use a local mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    fn require_key(&self) -> Result<String, ProviderError> {
        self.api_keys
            .try_get_api_key(PROVIDER_ID)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| ProviderError::auth("No Mistral API key configured - paste one or sign in in Settings → Mistral.").user_facing())
    }
}

#[async_trait]
impl LlmProvider for MistralProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let key = self.require_key()?;
        let request = self.client.get(format!("{}/models", self.base_url)).bearer_auth(key);

        let response = send(request, "Mistral").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        let body = response.text().await.map_err(|e| transport_error("Mistral", &e))?;

        let parsed: wire::MistralListModelsResponse = serde_json::from_str(&body)
            .map_err(|e| ProviderError::unknown(format!("Failed to parse Mistral model list: {e}")))?;
        let mut ids: Vec<String> = parsed.data.unwrap_or_default().into_iter().filter(|m| m.can_chat()).map(|m| m.id).collect();
        // The listing repeats a model under its dated id and its "-latest" alias.
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        let key = self.require_key()?;
        let json = serde_json::to_string(&wire::to_mistral_request(&request))
            .map_err(|e| ProviderError::unknown(format!("Failed to serialize Mistral request: {e}")))?;

        let http_request = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(key)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .body(json);

        let response = send(http_request, "Mistral").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        Ok(openai_compat::read_chunks(response.bytes_stream(), "Mistral").boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRole};
    use crate::providers::error::ProviderErrorKind;
    use crate::providers::keys::StaticApiKey;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(server: &MockServer, key: Option<&str>) -> MistralProvider {
        MistralProvider::new(reqwest::Client::new(), Arc::new(StaticApiKey(key.map(String::from)))).with_base_url(server.uri())
    }

    fn request(model: &str, text: &str) -> LlmRequest {
        LlmRequest {
            model: model.into(),
            system_instruction: None,
            history: vec![LlmMessage::new(LlmRole::User, text)],
            generation_config: LlmGenerationConfig::default(),
            enable_web_search: None,
        }
    }

    async fn collect(p: &MistralProvider, text: &str) -> Result<Vec<crate::providers::LlmResponseChunk>, ProviderError> {
        let stream = p.stream_completion(request("mistral-large-latest", text)).await?;
        stream.collect::<Vec<_>>().await.into_iter().collect()
    }

    #[tokio::test]
    async fn streaming_sends_the_bearer_token_and_yields_text_and_usage() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1,\"total_tokens\":4}}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer key-1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;

        let chunks = collect(&provider(&server, Some("key-1")), "Hello").await.unwrap();

        assert_eq!(chunks[0].delta_text, "Hi");
        let last = chunks.last().unwrap();
        assert_eq!(last.finish_reason.as_deref(), Some("stop"));
        assert_eq!(last.usage.as_ref().unwrap().total_tokens, Some(4));
        let body = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(body.contains("\"model\":\"mistral-large-latest\""), "{body}");
    }

    #[tokio::test]
    async fn missing_key_is_a_user_facing_auth_error_without_any_http_call() {
        let server = MockServer::start().await;

        let err = collect(&provider(&server, None), "Hello").await.unwrap_err();

        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(err.message_is_user_facing);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn error_responses_use_mistrals_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_raw(r#"{"object":"error","message":"Rate limit exceeded","type":"rate_limit"}"#, "application/json"))
            .mount(&server)
            .await;

        let err = collect(&provider(&server, Some("key")), "Hello").await.unwrap_err();

        assert_eq!(err.kind, ProviderErrorKind::RateLimit);
        assert_eq!(err.message, "Rate limit exceeded");
    }

    #[tokio::test]
    async fn list_models_keeps_chat_models_sorted_and_deduplicated() {
        let server = MockServer::start().await;
        let body = r#"{"data":[
            {"id":"mistral-small-latest","capabilities":{"completion_chat":true}},
            {"id":"mistral-embed","capabilities":{"completion_chat":false}},
            {"id":"codestral-latest","capabilities":{"completion_chat":true}},
            {"id":"mistral-small-latest","capabilities":{"completion_chat":true}}
        ]}"#;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer key"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .expect(1)
            .mount(&server)
            .await;

        let models = provider(&server, Some("key")).list_models().await.unwrap();

        assert_eq!(models, ["codestral-latest", "mistral-small-latest"]);
    }

    #[tokio::test]
    async fn list_models_without_a_key_makes_no_http_call() {
        let server = MockServer::start().await;
        let err = provider(&server, None).list_models().await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
