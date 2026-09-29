//! Grok via a SuperGrok / X Premium subscription login (OAuth) rather than a per-token API key.
//! Talks to the same OpenAI-compatible endpoint as the API-key product, with the OAuth access token
//! as the bearer.

mod errors;
pub mod wire;

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::StatusCode;

use super::auth::AccessTokenSource;
use super::error::ProviderError;
use super::http::{ensure_success, send, transport_error};
use super::{openai_compat, ChunkStream, LlmProvider};
use crate::llm::LlmRequest;

pub const PROVIDER_ID: &str = "xai-subscription";
const DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";

/// Models that are part of the subscription but may be missing from the live /models listing
/// (grok-build-0.1 is subscription-only and not in the API-key catalog). Only ever added to a
/// successful live list - never used in place of one, except when the endpoint doesn't exist.
const SUBSCRIPTION_ONLY_MODELS: [&str; 1] = ["grok-build-0.1"];

pub struct XaiSubscriptionProvider {
    client: reqwest::Client,
    auth: Arc<dyn AccessTokenSource>,
    base_url: String,
}

impl XaiSubscriptionProvider {
    pub fn new(client: reqwest::Client, auth: Arc<dyn AccessTokenSource>) -> Self {
        Self { client, auth, base_url: DEFAULT_BASE_URL.to_string() }
    }

    /// Points the provider at another server (tests use a local mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }
}

#[async_trait]
impl LlmProvider for XaiSubscriptionProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let token = self.auth.access_token().await?;
        let request = self.client.get(format!("{}/models", self.base_url)).bearer_auth(token);

        let response = send(request, "xAI").await?;
        let status = response.status();
        let body = response.text().await.map_err(|e| transport_error("xAI", &e))?;

        if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
            return Ok(SUBSCRIPTION_ONLY_MODELS.map(String::from).to_vec());
        }
        if !status.is_success() {
            return Err(errors::to_provider_error(status, &body));
        }

        let parsed: wire::XaiListModelsResponse = serde_json::from_str(&body)
            .map_err(|e| ProviderError::unknown(format!("Failed to parse xAI model list: {e}")))?;
        let mut models: Vec<String> = parsed.data.unwrap_or_default().into_iter().map(|m| m.id).collect();
        for extra in SUBSCRIPTION_ONLY_MODELS {
            if !models.iter().any(|m| m.eq_ignore_ascii_case(extra)) {
                models.push(extra.to_string());
            }
        }
        Ok(models)
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        let token = self.auth.access_token().await?;
        let json = serde_json::to_string(&wire::to_xai_request(&request))
            .map_err(|e| ProviderError::unknown(format!("Failed to serialize xAI request: {e}")))?;

        let http_request = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(json);

        let response = send(http_request, "xAI").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        Ok(openai_compat::read_chunks(response.bytes_stream(), "xAI").boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRole};
    use crate::providers::auth::StaticAccessToken;
    use crate::providers::error::ProviderErrorKind;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(server: &MockServer) -> XaiSubscriptionProvider {
        XaiSubscriptionProvider::new(reqwest::Client::new(), Arc::new(StaticAccessToken::new("AT")))
            .with_base_url(server.uri())
    }

    fn request() -> LlmRequest {
        LlmRequest {
            model: "grok-4.6".into(),
            system_instruction: Some("sys".into()),
            history: vec![LlmMessage::new(LlmRole::User, "hi")],
            generation_config: LlmGenerationConfig::default(),
            enable_web_search: None,
        }
    }

    async fn drain(p: &XaiSubscriptionProvider) -> Result<String, ProviderError> {
        let mut stream = p.stream_completion(request()).await?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            text.push_str(&chunk?.delta_text);
        }
        Ok(text)
    }

    #[tokio::test]
    async fn streaming_sends_the_bearer_token_and_yields_chunks() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer AT"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n",
                "text/event-stream",
            ))
            .expect(1)
            .mount(&server)
            .await;

        assert_eq!(drain(&provider(&server)).await.unwrap(), "Hi");
    }

    #[tokio::test]
    async fn a_403_is_a_helpful_user_facing_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(403).set_body_raw("{}", "application/json")).mount(&server).await;

        let err = drain(&provider(&server)).await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(err.message_is_user_facing);
        assert!(err.message.contains("API key"));
    }

    #[tokio::test]
    async fn a_token_failure_stops_the_request_before_any_http_call() {
        struct SignedOut;
        #[async_trait]
        impl AccessTokenSource for SignedOut {
            async fn access_token(&self) -> Result<String, ProviderError> {
                Err(ProviderError::auth("Not signed in.").user_facing())
            }
        }
        let server = MockServer::start().await;
        let p = XaiSubscriptionProvider::new(reqwest::Client::new(), Arc::new(SignedOut)).with_base_url(server.uri());

        assert_eq!(drain(&p).await.unwrap_err().message, "Not signed in.");
        assert!(p.list_models().await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_models_adds_subscription_only_models_to_the_live_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer AT"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"data":[{"id":"grok-4.6"}]}"#, "application/json"))
            .mount(&server)
            .await;

        assert_eq!(provider(&server).list_models().await.unwrap(), ["grok-4.6", "grok-build-0.1"]);
    }

    #[tokio::test]
    async fn list_models_does_not_duplicate_a_model_already_listed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"data":[{"id":"GROK-BUILD-0.1"}]}"#, "application/json"))
            .mount(&server)
            .await;

        assert_eq!(provider(&server).list_models().await.unwrap(), ["GROK-BUILD-0.1"]);
    }

    #[tokio::test]
    async fn list_models_falls_back_when_the_endpoint_is_missing() {
        for status in [404, 405] {
            let server = MockServer::start().await;
            Mock::given(method("GET")).respond_with(ResponseTemplate::new(status).set_body_raw("{}", "application/json")).mount(&server).await;
            assert_eq!(provider(&server).list_models().await.unwrap(), ["grok-build-0.1"], "{status}");
        }
    }

    #[tokio::test]
    async fn list_models_surfaces_real_errors_instead_of_the_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_raw(r#"{"error":"bad token"}"#, "application/json"))
            .mount(&server)
            .await;

        let err = provider(&server).list_models().await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert_eq!(err.message, "bad token");
    }
}
