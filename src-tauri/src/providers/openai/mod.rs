//! GPT models via a ChatGPT subscription login (OAuth) - the same Codex backend the Codex CLI uses,
//! speaking the Responses API, rather than the per-token OpenAI API.

mod errors;
mod stream;
pub mod wire;

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;

use super::auth::AccessTokenSource;
use super::error::ProviderError;
use super::http::{ensure_success, send, transport_error};
use super::oauth::OAuthClientConfig;
use super::{ChunkStream, LlmProvider};
use crate::llm::LlmRequest;

pub const PROVIDER_ID: &str = "chatgpt-subscription";
const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

/// Sent as the `originator` (authorize URL and API requests) so OpenAI can tell which app is calling.
pub const ORIGINATOR: &str = "localtavern";

/// Where the refresh token is kept in the credential store.
pub const SECRET_KEY: &str = "chatgpt-subscription";

/// OpenAI's OAuth client settings for a ChatGPT (Plus/Pro/Business) login. The client id is the
/// Codex CLI's public client, which only allows these exact loopback redirects - any other host,
/// port or path makes auth.openai.com reject the authorize request outright.
pub fn oauth_config() -> OAuthClientConfig {
    OAuthClientConfig {
        display_name: "ChatGPT".into(),
        authorize_url: "https://auth.openai.com/oauth/authorize".into(),
        token_url: "https://auth.openai.com/oauth/token".into(),
        client_id: "app_EMoamEEZ73f0CkXaXp7hrann".into(),
        // Deliberately minimal: extra scopes only inflate the JWTs.
        scope: "openid profile email offline_access".into(),
        redirect_host: "localhost".into(),
        callback_ports: vec![1455, 1457],
        callback_path: "/auth/callback".into(),
        extra_authorize_params: vec![
            ("id_token_add_organizations".into(), "true".into()),
            ("codex_cli_simplified_flow".into(), "true".into()),
            ("originator".into(), ORIGINATOR.into()),
        ],
        send_nonce: false,
    }
}

/// The model catalog hides models whose minimum client version exceeds this, so it's set high
/// enough to list everything current.
const CATALOG_CLIENT_VERSION: &str = "0.999.0";

pub struct ChatGptSubscriptionProvider {
    client: reqwest::Client,
    auth: Arc<dyn AccessTokenSource>,
    base_url: String,
}

impl ChatGptSubscriptionProvider {
    pub fn new(client: reqwest::Client, auth: Arc<dyn AccessTokenSource>) -> Self {
        Self { client, auth, base_url: DEFAULT_BASE_URL.to_string() }
    }

    /// Points the provider at another server (tests use a local mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    async fn authorized(&self, builder: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder, ProviderError> {
        let token = self.auth.access_token().await?;
        let mut builder = builder
            .bearer_auth(token)
            .header("originator", ORIGINATOR)
            .header("OpenAI-Beta", "responses=experimental");
        if let Some(account_id) = self.auth.account_id().filter(|a| !a.is_empty()) {
            builder = builder.header("chatgpt-account-id", account_id);
        }
        Ok(builder)
    }
}

#[async_trait]
impl LlmProvider for ChatGptSubscriptionProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let request = self
            .authorized(self.client.get(format!("{}/models?client_version={CATALOG_CLIENT_VERSION}", self.base_url)))
            .await?
            .header("accept", "application/json");

        let response = send(request, "ChatGPT").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        let body = response.text().await.map_err(|e| transport_error("ChatGPT", &e))?;

        let parsed: wire::CodexModelsResponse = serde_json::from_str(&body)
            .map_err(|e| ProviderError::unknown(format!("Failed to parse ChatGPT model list: {e}")))?;

        let mut listed: Vec<_> = parsed
            .models
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.visibility.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("list")))
            .collect();
        listed.sort_by_key(|m| m.priority.unwrap_or(i32::MAX)); // stable
        Ok(listed.into_iter().map(|m| m.slug).collect())
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        let json = serde_json::to_string(&wire::to_responses_request(&request))
            .map_err(|e| ProviderError::unknown(format!("Failed to serialize ChatGPT request: {e}")))?;

        let http_request = self
            .authorized(self.client.post(format!("{}/responses", self.base_url)))
            .await?
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .body(json);

        let response = send(http_request, "ChatGPT").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        Ok(stream::read_chunks(response.bytes_stream()).boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRole};
    use crate::providers::auth::StaticAccessToken;
    use crate::providers::error::ProviderErrorKind;
    use wiremock::matchers::{header, header_exists, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(server: &MockServer, auth: StaticAccessToken) -> ChatGptSubscriptionProvider {
        ChatGptSubscriptionProvider::new(reqwest::Client::new(), Arc::new(auth)).with_base_url(server.uri())
    }

    fn request() -> LlmRequest {
        LlmRequest {
            model: "gpt-5-codex".into(),
            system_instruction: Some("be brief".into()),
            history: vec![LlmMessage::new(LlmRole::User, "hi")],
            generation_config: LlmGenerationConfig::default(),
            enable_web_search: None,
        }
    }

    async fn drain(p: &ChatGptSubscriptionProvider) -> Result<String, ProviderError> {
        let mut stream = p.stream_completion(request()).await?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            text.push_str(&chunk?.delta_text);
        }
        Ok(text)
    }

    const HI: &str = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{}}",
    );

    #[tokio::test]
    async fn streaming_sends_bearer_account_and_originator_headers_to_codex_responses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .and(header("authorization", "Bearer AT"))
            .and(header("chatgpt-account-id", "acc-1"))
            .and(header("originator", "localtavern"))
            .and(header("openai-beta", "responses=experimental"))
            .and(header("accept", "text/event-stream"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(HI, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;

        let text = drain(&provider(&server, StaticAccessToken::with_account("AT", "acc-1"))).await.unwrap();
        assert_eq!(text, "Hi");

        let body = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(body.contains("\"instructions\":\"be brief\""), "{body}");
        assert!(body.contains("\"store\":false"), "{body}");
    }

    #[tokio::test]
    async fn the_account_header_is_omitted_when_unknown() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("data: {\"type\":\"response.completed\",\"response\":{}}", "text/event-stream"))
            .mount(&server)
            .await;

        drain(&provider(&server, StaticAccessToken::new("AT"))).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        assert!(!requests[0].headers.contains_key("chatgpt-account-id"));
    }

    #[tokio::test]
    async fn a_401_is_a_user_facing_auth_error_with_a_sign_in_hint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_raw(r#"{"detail":"Unauthorized"}"#, "application/json"))
            .mount(&server)
            .await;

        let err = drain(&provider(&server, StaticAccessToken::new("AT"))).await.unwrap_err();

        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(err.message_is_user_facing);
        assert!(err.message.contains("Unauthorized"));
        assert!(err.message.contains("Settings"));
    }

    #[tokio::test]
    async fn a_429_is_a_rate_limit_error_with_the_backends_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_raw(
                r#"{"error":{"message":"You've hit your usage limit."}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let err = drain(&provider(&server, StaticAccessToken::new("AT"))).await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::RateLimit);
        assert_eq!(err.message, "You've hit your usage limit.");
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
        let p = ChatGptSubscriptionProvider::new(reqwest::Client::new(), Arc::new(SignedOut)).with_base_url(server.uri());
        let err = drain(&p).await.unwrap_err();

        assert_eq!(err.message, "Not signed in.");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_models_returns_visible_models_in_priority_order() {
        let server = MockServer::start().await;
        let body = r#"{"models":[
            {"slug":"b-model","visibility":"list","priority":2},
            {"slug":"hidden","visibility":"hide","priority":0},
            {"slug":"a-model","visibility":"LIST","priority":1},
            {"slug":"unranked","visibility":"list"}
        ]}"#;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("client_version", CATALOG_CLIENT_VERSION))
            .and(header_exists("authorization"))
            .and(header("accept", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .expect(1)
            .mount(&server)
            .await;

        let models = provider(&server, StaticAccessToken::new("AT")).list_models().await.unwrap();
        assert_eq!(models, ["a-model", "b-model", "unranked"]);
    }
}
