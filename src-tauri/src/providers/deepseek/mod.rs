//! DeepSeek (`api.deepseek.com`), an OpenAI-compatible API authenticated with a bearer API key.

mod errors;
pub mod wire;

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;

use super::error::ProviderError;
use super::http::{ensure_success, send, transport_error};
use super::keys::ApiKeyProvider;
use super::{openai_compat, ChunkStream, LlmProvider};
use crate::llm::LlmRequest;

pub const PROVIDER_ID: &str = "deepseek";
const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";

pub struct DeepSeekProvider {
    client: reqwest::Client,
    api_keys: Arc<dyn ApiKeyProvider>,
    base_url: String,
}

impl DeepSeekProvider {
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
            .ok_or_else(|| ProviderError::auth("No DeepSeek API key configured."))
    }
}

#[async_trait]
impl LlmProvider for DeepSeekProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let key = self.require_key()?;
        let request = self.client.get(format!("{}/models", self.base_url)).bearer_auth(key);

        let response = send(request, "DeepSeek").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        let body = response.text().await.map_err(|e| transport_error("DeepSeek", &e))?;

        let parsed: wire::DeepSeekListModelsResponse = serde_json::from_str(&body)
            .map_err(|e| ProviderError::unknown(format!("Failed to parse DeepSeek model list: {e}")))?;
        Ok(parsed.data.unwrap_or_default().into_iter().map(|m| m.id).collect())
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        let key = self.require_key()?;
        let json = serde_json::to_string(&wire::to_deepseek_request(&request))
            .map_err(|e| ProviderError::unknown(format!("Failed to serialize DeepSeek request: {e}")))?;

        let http_request = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(key)
            .header("content-type", "application/json")
            .body(json);

        let response = send(http_request, "DeepSeek").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        Ok(openai_compat::read_chunks(response.bytes_stream(), "DeepSeek").boxed())
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

    fn provider(server: &MockServer, key: Option<&str>) -> DeepSeekProvider {
        DeepSeekProvider::new(reqwest::Client::new(), Arc::new(StaticApiKey(key.map(String::from))))
            .with_base_url(server.uri())
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

    async fn collect(p: &DeepSeekProvider, model: &str, text: &str) -> Result<Vec<crate::providers::LlmResponseChunk>, ProviderError> {
        let stream = p.stream_completion(request(model, text)).await?;
        stream.collect::<Vec<_>>().await.into_iter().collect()
    }

    #[tokio::test]
    async fn streaming_sends_the_bearer_token_and_model_in_the_body() {
        let server = MockServer::start().await;
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer test-key-123"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;

        let chunks = collect(&provider(&server, Some("test-key-123")), "deepseek-chat", "Hello").await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta_text, "Hi");
        let body = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(body.contains("\"model\":\"deepseek-chat\""), "{body}");
        assert!(body.contains("\"stream\":true"), "{body}");
    }

    #[tokio::test]
    async fn missing_api_key_is_an_auth_error_without_any_http_call() {
        let server = MockServer::start().await;
        let err = collect(&provider(&server, None), "deepseek-chat", "Hello").await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rate_limit_responses_map_to_provider_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_raw(
                r#"{"error":{"message":"Rate limit reached","type":"rate_limit_error"}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let err = collect(&provider(&server, Some("key")), "deepseek-chat", "Hello").await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::RateLimit);
        assert_eq!(err.message, "Rate limit reached");
        assert_eq!(err.http_status, Some(429));
    }

    #[tokio::test]
    async fn get_completion_drains_the_stream_into_one_aggregated_chunk() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo!\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .mount(&server)
            .await;

        let result = provider(&server, Some("key")).get_completion(request("deepseek-chat", "Hello")).await.unwrap();

        assert_eq!(result.delta_text, "Hello!");
        assert!(result.is_final);
        assert_eq!(result.finish_reason.as_deref(), Some("stop"));
        assert_eq!(result.usage.unwrap().total_tokens, Some(5));
    }

    #[tokio::test]
    async fn list_models_returns_ids_and_sends_the_bearer_token() {
        let server = MockServer::start().await;
        let body = r#"{"object":"list","data":[{"id":"deepseek-chat","object":"model"},{"id":"deepseek-reasoner","object":"model"}]}"#;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer key"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .expect(1)
            .mount(&server)
            .await;

        let models = provider(&server, Some("key")).list_models().await.unwrap();
        assert_eq!(models, ["deepseek-chat", "deepseek-reasoner"]);
    }

    #[tokio::test]
    async fn list_models_returns_empty_for_an_empty_data_array() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"object":"list","data":[]}"#, "application/json"))
            .mount(&server)
            .await;
        assert!(provider(&server, Some("key")).list_models().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_models_without_a_key_is_an_auth_error_without_any_http_call() {
        let server = MockServer::start().await;
        let err = provider(&server, None).list_models().await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_models_maps_error_responses() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_raw(
                r#"{"error":{"message":"Authentication Fails","type":"authentication_error"}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let err = provider(&server, Some("bad")).list_models().await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert_eq!(err.message, "Authentication Fails");
    }
}
