//! Google Gemini (`generativelanguage.googleapis.com`), API-key authenticated.

mod errors;
mod stream;
pub mod wire;

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

pub use wire::{GeminiOptionsSource, GeminiRequestOptions, GeminiSafetyCategory, GeminiSafetyThreshold};

use super::error::ProviderError;
use super::http::{ensure_success, send};
use super::keys::ApiKeyProvider;
use super::{ChunkStream, LlmProvider};
use crate::llm::LlmRequest;

pub const PROVIDER_ID: &str = "gemini";
const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models/";

/// Escapes a model name for use as a URL path segment, leaving the RFC 3986 unreserved characters
/// (`-._~`) alone like .NET's `Uri.EscapeDataString`.
const MODEL_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

pub struct GeminiProvider {
    client: reqwest::Client,
    api_keys: Arc<dyn ApiKeyProvider>,
    options: Arc<dyn GeminiOptionsSource>,
    /// Ends with `/`; model names and `?pageSize` are appended directly.
    base_url: String,
}

impl GeminiProvider {
    pub fn new(
        client: reqwest::Client,
        api_keys: Arc<dyn ApiKeyProvider>,
        options: Arc<dyn GeminiOptionsSource>,
    ) -> Self {
        Self { client, api_keys, options, base_url: DEFAULT_BASE_URL.to_string() }
    }

    /// Points the provider at another server (tests use a local mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        let mut url = base_url.into();
        if !url.ends_with('/') {
            url.push('/');
        }
        self.base_url = url;
        self
    }

    fn require_key(&self) -> Result<String, ProviderError> {
        self.api_keys
            .try_get_api_key(PROVIDER_ID)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| ProviderError::auth("No Gemini API key configured."))
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let key = self.require_key()?;
        let request = self
            .client
            .get(format!("{}?pageSize=100", self.base_url))
            .header("x-goog-api-key", key);

        let response = send(request, "Gemini").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        let body = response.text().await.map_err(|e| super::http::transport_error("Gemini", &e))?;

        let parsed: wire::GeminiListModelsResponse = serde_json::from_str(&body)
            .map_err(|e| ProviderError::unknown(format!("Failed to parse Gemini model list: {e}")))?;

        Ok(parsed
            .models
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.supported_generation_methods.as_deref().is_some_and(|s| s.iter().any(|x| x == "generateContent")))
            .map(|m| m.name.strip_prefix("models/").map(String::from).unwrap_or(m.name))
            .collect())
    }

    async fn context_window(&self, model: &str) -> Option<i32> {
        let key = self.require_key().ok()?;
        let url = format!("{}{}", self.base_url, utf8_percent_encode(model, MODEL_SEGMENT));
        let response = send(self.client.get(url).header("x-goog-api-key", key), "Gemini").await.ok()?;
        let response = ensure_success(response, errors::to_provider_error).await.ok()?;
        let info: wire::GeminiModelInfo = serde_json::from_str(&response.text().await.ok()?).ok()?;
        // The prompt (the whole conversation) must fit the input limit; the reply has its own.
        info.input_token_limit
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        let key = self.require_key()?;
        let options = self.options.current();
        let model = request.model.clone();

        // Building the request reads attachment files off disk - keep that off the async threads.
        let json = tokio::task::spawn_blocking(move || {
            serde_json::to_string(&wire::to_gemini_request(&request, &options))
        })
        .await
        .map_err(|e| ProviderError::unknown(format!("Request mapping failed: {e}")))?
        .map_err(|e| ProviderError::unknown(format!("Failed to serialize Gemini request: {e}")))?;

        let url = format!(
            "{}{}:streamGenerateContent?alt=sse",
            self.base_url,
            utf8_percent_encode(&model, MODEL_SEGMENT)
        );
        let http_request = self
            .client
            .post(url)
            .header("x-goog-api-key", key)
            .header("content-type", "application/json")
            .body(json);

        let response = send(http_request, "Gemini").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;

        Ok(stream::read_chunks(response.bytes_stream()).boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRole};
    use crate::providers::error::ProviderErrorKind;
    use crate::providers::keys::StaticApiKey;
    use crate::providers::LlmResponseChunk;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const HI_SSE: &str =
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hi\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n";

    fn provider(server: &MockServer, key: Option<&str>) -> GeminiProvider {
        provider_with(server, key, GeminiRequestOptions::default())
    }

    fn provider_with(server: &MockServer, key: Option<&str>, options: GeminiRequestOptions) -> GeminiProvider {
        GeminiProvider::new(
            reqwest::Client::new(),
            Arc::new(StaticApiKey(key.map(String::from))),
            Arc::new(options),
        )
        .with_base_url(format!("{}/v1beta/models/", server.uri()))
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

    async fn collect(provider: &GeminiProvider, model: &str, text: &str) -> Result<Vec<LlmResponseChunk>, ProviderError> {
        let stream = provider.stream_completion(request(model, text)).await?;
        stream.collect::<Vec<_>>().await.into_iter().collect()
    }

    async fn last_body(server: &MockServer) -> String {
        let requests = server.received_requests().await.unwrap();
        String::from_utf8(requests.last().expect("a request was made").body.clone()).unwrap()
    }

    #[tokio::test]
    async fn streaming_sends_the_api_key_header_and_model_in_the_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-2.5-flash:streamGenerateContent"))
            .and(query_param("alt", "sse"))
            .and(header("x-goog-api-key", "test-key-123"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(HI_SSE, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;

        let chunks = collect(&provider(&server, Some("test-key-123")), "gemini-2.5-flash", "Hello").await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta_text, "Hi");
        assert!(last_body(&server).await.contains("\"contents\""));
    }

    #[tokio::test]
    async fn missing_api_key_is_an_auth_error_without_any_http_call() {
        let server = MockServer::start().await;
        let err = collect(&provider(&server, None), "gemini-2.5-flash", "Hello").await.unwrap_err();

        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rate_limit_responses_map_to_provider_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_raw(
                r#"{"error":{"code":429,"message":"Quota exceeded","status":"RESOURCE_EXHAUSTED"}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let err = collect(&provider(&server, Some("key")), "gemini-2.5-flash", "Hello").await.unwrap_err();

        assert_eq!(err.kind, ProviderErrorKind::RateLimit);
        assert_eq!(err.message, "Quota exceeded");
        assert_eq!(err.http_status, Some(429));
    }

    #[tokio::test]
    async fn auth_error_responses_map_to_provider_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403).set_body_raw(
                r#"{"error":{"code":403,"message":"API key not valid","status":"PERMISSION_DENIED"}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let err = collect(&provider(&server, Some("bad-key")), "gemini-2.5-flash", "Hello").await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
    }

    #[tokio::test]
    async fn unreachable_server_is_a_network_error() {
        let provider = GeminiProvider::new(
            reqwest::Client::new(),
            Arc::new(StaticApiKey::some("key")),
            Arc::new(GeminiRequestOptions::default()),
        )
        .with_base_url("http://127.0.0.1:1/v1beta/models/");
        let err = collect(&provider, "m", "Hello").await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Network);
    }

    #[tokio::test]
    async fn get_completion_drains_the_stream_into_one_aggregated_chunk() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hel\"}],\"role\":\"model\"}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"lo!\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":2,\"totalTokenCount\":5}}\n\n",
        );
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .mount(&server)
            .await;

        let result = provider(&server, Some("key")).get_completion(request("gemini-2.5-flash", "Hello")).await.unwrap();

        assert_eq!(result.delta_text, "Hello!");
        assert!(result.is_final);
        assert_eq!(result.finish_reason.as_deref(), Some("STOP"));
        assert_eq!(result.usage.unwrap().total_tokens, Some(5));
    }

    #[tokio::test]
    async fn all_categories_off_are_sent_as_explicit_off_thresholds_on_the_wire() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(HI_SSE, "text/event-stream"))
            .mount(&server)
            .await;

        collect(&provider(&server, Some("key")), "gemini-3.1-pro-preview", "Hello").await.unwrap();

        let body = last_body(&server).await;
        assert!(body.contains("\"safetySettings\":["), "{body}");
        for category in ["HARASSMENT", "HATE_SPEECH", "SEXUALLY_EXPLICIT", "DANGEROUS_CONTENT"] {
            assert!(
                body.contains(&format!("{{\"category\":\"HARM_CATEGORY_{category}\",\"threshold\":\"OFF\"}}")),
                "{category}: {body}"
            );
        }
    }

    #[tokio::test]
    async fn request_options_are_read_fresh_for_each_call() {
        struct Toggle(std::sync::atomic::AtomicBool);
        impl GeminiOptionsSource for Toggle {
            fn current(&self) -> GeminiRequestOptions {
                GeminiRequestOptions {
                    enable_google_search: self.0.load(std::sync::atomic::Ordering::SeqCst),
                    ..Default::default()
                }
            }
        }

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(HI_SSE, "text/event-stream"))
            .mount(&server)
            .await;
        let toggle = Arc::new(Toggle(std::sync::atomic::AtomicBool::new(false)));
        let provider = GeminiProvider::new(reqwest::Client::new(), Arc::new(StaticApiKey::some("k")), toggle.clone())
            .with_base_url(format!("{}/v1beta/models/", server.uri()));

        collect(&provider, "m", "a").await.unwrap();
        assert!(!last_body(&server).await.contains("google_search"));

        toggle.0.store(true, std::sync::atomic::Ordering::SeqCst);
        collect(&provider, "m", "b").await.unwrap();
        assert!(last_body(&server).await.contains("google_search"));
    }

    // ---- list_models ----

    #[tokio::test]
    async fn list_models_returns_generate_content_models_without_the_models_prefix() {
        let server = MockServer::start().await;
        let body = r#"{
            "models": [
              { "name": "models/gemini-2.5-flash", "supportedGenerationMethods": ["generateContent", "countTokens"] },
              { "name": "models/embedding-001", "supportedGenerationMethods": ["embedContent"] },
              { "name": "models/gemini-2.5-pro", "supportedGenerationMethods": ["generateContent"] },
              { "name": "models/no-methods" }
            ]
        }"#;
        Mock::given(method("GET"))
            .and(path("/v1beta/models/"))
            .and(query_param("pageSize", "100"))
            .and(header("x-goog-api-key", "key"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .expect(1)
            .mount(&server)
            .await;

        let models = provider(&server, Some("key")).list_models().await.unwrap();
        assert_eq!(models, ["gemini-2.5-flash", "gemini-2.5-pro"]);
    }

    #[tokio::test]
    async fn list_models_returns_empty_when_there_are_no_usable_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"models":[]}"#, "application/json"))
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
            .respond_with(ResponseTemplate::new(403).set_body_raw(
                r#"{"error":{"code":403,"message":"API key not valid","status":"PERMISSION_DENIED"}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let err = provider(&server, Some("bad")).list_models().await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
    }
}
