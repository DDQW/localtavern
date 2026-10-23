//! A local Ollama server (https://ollama.com) over its native REST API. No API key - just a base
//! URL, which defaults to Ollama's standard localhost:11434 and is user-configurable in case Ollama
//! runs elsewhere (a different port, a machine on the LAN, WSL, etc.).

mod errors;
mod stream;
pub mod wire;

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;

pub use wire::{OllamaOptionsSource, OllamaRequestOptions};

use super::error::ProviderError;
use super::http::ensure_success;
use super::{ChunkStream, LlmProvider};
use crate::gpu::GpuGate;
use crate::llm::LlmRequest;

pub const PROVIDER_ID: &str = "ollama";

pub struct OllamaProvider {
    client: reqwest::Client,
    options: Arc<dyn OllamaOptionsSource>,
    gpu: GpuGate,
}

impl OllamaProvider {
    pub fn new(client: reqwest::Client, options: Arc<dyn OllamaOptionsSource>) -> Self {
        Self { client, options, gpu: GpuGate::default() }
    }

    /// Replies wait while a picture holds `gpu` (see `crate::gpu`).
    pub fn with_gpu_gate(mut self, gpu: GpuGate) -> Self {
        self.gpu = gpu;
        self
    }

    /// Sends a request; an unreachable server becomes the friendly "is it running?" error.
    async fn send(&self, request: reqwest::RequestBuilder, base_url: &str) -> Result<reqwest::Response, ProviderError> {
        request.send().await.map_err(|e| {
            if e.is_timeout() {
                ProviderError::network(format!("Ollama request timed out: {e}"))
            } else {
                errors::connection_failed(base_url)
            }
        })
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let options = self.options.current();
        let base_url = options.base_url.trim_end_matches('/').to_string();

        let response = self.send(self.client.get(format!("{base_url}/api/tags")), &base_url).await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        let body = response.text().await.map_err(|_| errors::connection_failed(&base_url))?;

        let parsed: wire::OllamaTagsResponse = serde_json::from_str(&body)
            .map_err(|e| ProviderError::unknown(format!("Failed to parse Ollama model list: {e}")))?;
        Ok(parsed.models.unwrap_or_default().into_iter().map(|m| m.name).collect())
    }

    /// Whatever the model could hold, Ollama only gives it the `num_ctx` this app sends.
    async fn context_window(&self, _model: &str) -> Option<i32> {
        Some(self.options.current().context_size)
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        self.gpu.free().await;
        let options = self.options.current();
        let base_url = options.base_url.trim_end_matches('/').to_string();
        let mut body = wire::to_ollama_request(&request, Some(options.context_size));

        let response = match self.post_chat(&body, &base_url).await {
            // A thinking level on a model whose template doesn't declare thinking (e.g. Gemma) is a
            // hard 400. Retry without `think` so the model just answers in its default mode.
            Err(e) if body.think.is_some() && errors::is_thinking_unsupported(&e) => {
                body.think = None;
                self.post_chat(&body, &base_url).await?
            }
            Err(e) if body.think.is_some() => {
                let asked = body.think.as_ref().and_then(|t| t.as_str()).unwrap_or_default().to_string();
                match errors::retry_think_level(&e, &asked) {
                    Some(level) => {
                        body.think = level.map(serde_json::Value::from);
                        self.post_chat(&body, &base_url).await?
                    }
                    None => return Err(e),
                }
            }
            other => other?,
        };
        Ok(stream::read_chunks(response.bytes_stream()).boxed())
    }
}

impl OllamaProvider {
    async fn post_chat(&self, body: &wire::OllamaChatRequest, base_url: &str) -> Result<reqwest::Response, ProviderError> {
        let json = serde_json::to_string(body)
            .map_err(|e| ProviderError::unknown(format!("Failed to serialize Ollama request: {e}")))?;
        let http_request = self
            .client
            .post(format!("{base_url}/api/chat"))
            .header("content-type", "application/json")
            .body(json);

        let response = self.send(http_request, base_url).await?;
        ensure_success(response, errors::to_provider_error).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRole};
    use crate::providers::error::ProviderErrorKind;
    use crate::providers::LlmResponseChunk;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(base_url: &str) -> OllamaProvider {
        OllamaProvider::new(reqwest::Client::new(), Arc::new(OllamaRequestOptions::new(base_url)))
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

    async fn collect(p: &OllamaProvider, model: &str) -> Result<Vec<LlmResponseChunk>, ProviderError> {
        let stream = p.stream_completion(request(model, "Hello")).await?;
        stream.collect::<Vec<_>>().await.into_iter().collect()
    }

    #[tokio::test]
    async fn streaming_posts_to_the_chat_endpoint_with_model_stream_and_num_ctx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "{\"message\":{\"content\":\"Hi\"},\"done\":true,\"done_reason\":\"stop\"}\n",
                "application/x-ndjson",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let chunks = collect(&provider(&server.uri()), "llama3").await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta_text, "Hi");
        let body = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(body.contains("\"model\":\"llama3\""), "{body}");
        assert!(body.contains("\"stream\":true"), "{body}");
        assert!(body.contains(&format!("\"num_ctx\":{}", wire::DEFAULT_CONTEXT_SIZE)), "{body}");
    }

    #[tokio::test]
    async fn a_thinking_level_on_a_model_without_thinking_is_retried_without_think() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"think\""))
            .respond_with(ResponseTemplate::new(400).set_body_raw(r#"{"error":"\"gemma4\" does not support thinking"}"#, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("{\"message\":{\"content\":\"Hi\"},\"done\":true}\n", "application/x-ndjson"))
            .expect(1)
            .mount(&server)
            .await;
        let mut req = request("gemma4", "Hello");
        req.generation_config.reasoning_effort = Some(crate::models::ReasoningEffort::High);

        let chunks: Vec<_> = provider(&server.uri()).stream_completion(req).await.unwrap().collect().await;

        assert_eq!(chunks[0].as_ref().unwrap().delta_text, "Hi");
    }

    #[tokio::test]
    async fn a_template_that_rejects_the_thinking_level_is_retried_with_one_it_lists() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"think\":\"high\""))
            .respond_with(ResponseTemplate::new(500).set_body_raw(
                r#"{"error":"Jinja Exception: Unexpected reasoning effort high. Supported types are xhigh (default), medium, and low."}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"think\":\"xhigh\""))
            .respond_with(ResponseTemplate::new(200).set_body_raw("{\"message\":{\"content\":\"Hi\"},\"done\":true}
", "application/x-ndjson"))
            .expect(1)
            .mount(&server)
            .await;
        let mut req = request("qwen", "Hello");
        req.generation_config.reasoning_effort = Some(crate::models::ReasoningEffort::High);

        let chunks: Vec<_> = provider(&server.uri()).stream_completion(req).await.unwrap().collect().await;

        assert_eq!(chunks[0].as_ref().unwrap().delta_text, "Hi");
    }

    #[tokio::test]
    async fn other_bad_requests_are_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_raw(r#"{"error":"invalid options"}"#, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        let mut req = request("gemma4", "Hello");
        req.generation_config.reasoning_effort = Some(crate::models::ReasoningEffort::High);

        let err = provider(&server.uri()).stream_completion(req).await.err().unwrap();
        assert_eq!(err.message, "invalid options");
    }

    #[tokio::test]
    async fn trailing_slashes_on_the_configured_base_url_are_ignored() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("{\"message\":{\"content\":\"Hi\"},\"done\":true}\n", "application/x-ndjson"))
            .expect(1)
            .mount(&server)
            .await;

        collect(&provider(&format!("{}//", server.uri())), "llama3").await.unwrap();
    }

    #[tokio::test]
    async fn a_configured_context_size_is_sent_as_num_ctx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("{\"message\":{\"content\":\"\"},\"done\":true}\n", "application/x-ndjson"))
            .mount(&server)
            .await;
        let options = OllamaRequestOptions { base_url: server.uri(), context_size: 32768 };
        let p = OllamaProvider::new(reqwest::Client::new(), Arc::new(options));

        collect(&p, "llama3").await.unwrap();

        let body = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(body.contains("\"num_ctx\":32768"), "{body}");
    }

    #[tokio::test]
    async fn not_found_maps_to_an_invalid_request_error_with_ollamas_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404).set_body_raw(r#"{"error":"model 'ghost' not found"}"#, "application/json"))
            .mount(&server)
            .await;

        let err = collect(&provider(&server.uri()), "ghost").await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::InvalidRequest);
        assert_eq!(err.message, "model 'ghost' not found");
    }

    #[tokio::test]
    async fn an_unreachable_server_gets_a_friendly_message() {
        let err = collect(&provider("http://127.0.0.1:1"), "llama3").await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Network);
        assert!(err.message.contains("127.0.0.1:1"), "{}", err.message);
        assert!(err.message.contains("Is it running?"));

        let err = provider("http://127.0.0.1:1").list_models().await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Network);
    }

    #[tokio::test]
    async fn get_completion_drains_the_stream_into_one_aggregated_chunk() {
        let server = MockServer::start().await;
        let ndjson = concat!(
            "{\"message\":{\"content\":\"Hel\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"lo!\"},\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":3,\"eval_count\":2}\n",
        );
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(ndjson, "application/x-ndjson"))
            .mount(&server)
            .await;

        let result = provider(&server.uri()).get_completion(request("llama3", "Hello")).await.unwrap();

        assert_eq!(result.delta_text, "Hello!");
        assert!(result.is_final);
        assert_eq!(result.finish_reason.as_deref(), Some("stop"));
        assert_eq!(result.usage.unwrap().total_tokens, Some(5));
    }

    #[tokio::test]
    async fn list_models_returns_names_from_the_tags_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"models":[{"name":"llama3:8b"},{"name":"mistral:latest"}]}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        assert_eq!(provider(&server.uri()).list_models().await.unwrap(), ["llama3:8b", "mistral:latest"]);
    }

    #[tokio::test]
    async fn list_models_returns_empty_when_there_are_no_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"models":[]}"#, "application/json"))
            .mount(&server)
            .await;
        assert!(provider(&server.uri()).list_models().await.unwrap().is_empty());
    }
}
