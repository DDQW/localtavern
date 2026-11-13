//! A local [oMLX](https://github.com/jundot/omlx) server: MLX inference on Apple Silicon, spoken as
//! the OpenAI chat API. No key on loopback. A key is sent only when one has been saved, which is
//! what oMLX requires once it listens beyond this machine.

mod errors;
pub mod wire;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::StreamExt;

pub use wire::{OmlxOptionsSource, OmlxRequestOptions};

use super::error::ProviderError;
use super::http::ensure_success;
use super::keys::ApiKeyProvider;
use super::{openai_compat, ChunkStream, LlmProvider};
use crate::gpu::GpuGate;
use crate::llm::LlmRequest;

pub const PROVIDER_ID: &str = "omlx";

pub struct OmlxProvider {
    client: reqwest::Client,
    api_keys: Arc<dyn ApiKeyProvider>,
    options: Arc<dyn OmlxOptionsSource>,
    gpu: GpuGate,
    /// Context lengths from the last model listing, by model id.
    context_windows: Mutex<HashMap<String, i32>>,
}

impl OmlxProvider {
    pub fn new(
        client: reqwest::Client,
        api_keys: Arc<dyn ApiKeyProvider>,
        options: Arc<dyn OmlxOptionsSource>,
    ) -> Self {
        Self {
            client,
            api_keys,
            options,
            gpu: GpuGate::default(),
            context_windows: Mutex::new(HashMap::new()),
        }
    }

    /// Replies wait while a picture holds `gpu`. oMLX uses the same Apple GPU.
    pub fn with_gpu_gate(mut self, gpu: GpuGate) -> Self {
        self.gpu = gpu;
        self
    }

    fn root(&self) -> String {
        wire::server_root(&self.options.current().base_url)
    }

    fn authed(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self
            .api_keys
            .try_get_api_key(PROVIDER_ID)
            .filter(|k| !k.is_empty())
        {
            Some(key) => request.bearer_auth(key),
            None => request,
        }
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        root: &str,
    ) -> Result<reqwest::Response, ProviderError> {
        self.authed(request).send().await.map_err(|e| {
            if e.is_timeout() {
                ProviderError::network(format!("oMLX request timed out: {e}"))
            } else {
                errors::connection_failed(root)
            }
        })
    }

    async fn post_chat(
        &self,
        body: &wire::OmlxChatCompletionRequest,
        root: &str,
    ) -> Result<reqwest::Response, ProviderError> {
        let json = serde_json::to_string(body).map_err(|e| {
            ProviderError::unknown(format!("Failed to serialize oMLX request: {e}"))
        })?;
        let response = self
            .send(
                self.client
                    .post(format!("{root}/v1/chat/completions"))
                    .header("content-type", "application/json")
                    .body(json),
                root,
            )
            .await?;
        ensure_success(response, errors::to_provider_error).await
    }
}

#[async_trait]
impl LlmProvider for OmlxProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let root = self.root();
        let response = self
            .send(self.client.get(format!("{root}/v1/models")), &root)
            .await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        let body = response
            .text()
            .await
            .map_err(|_| errors::connection_failed(&root))?;
        let parsed: wire::OmlxListModelsResponse = serde_json::from_str(&body)
            .map_err(|e| ProviderError::unknown(format!("Failed to parse oMLX model list: {e}")))?;
        let models = parsed.data.unwrap_or_default();

        let mut windows = HashMap::new();
        for model in &models {
            if let Some(len) = model
                .max_model_len
                .and_then(|n| i32::try_from(n).ok())
                .filter(|n| *n > 0)
            {
                windows.insert(model.id.clone(), len);
            }
        }
        *self.context_windows.lock().unwrap() = windows;

        let chat: Vec<String> = models
            .iter()
            .filter(|m| m.can_chat())
            .map(|m| m.id.clone())
            .collect();
        // An unexpected `model_type` must not hide the whole catalog.
        if chat.is_empty() {
            Ok(models.into_iter().map(|m| m.id).collect())
        } else {
            Ok(chat)
        }
    }

    async fn context_window(&self, model: &str) -> Option<i32> {
        self.context_windows.lock().unwrap().get(model).copied()
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        self.gpu.free().await;
        let root = self.root();
        let mut body = wire::to_omlx_request(&request);
        let response = match self.post_chat(&body, &root).await {
            Err(e) if wire::asks_to_think(&body) => match errors::think_retry(&e) {
                Some(errors::ThinkRetry::Level(level)) => {
                    body.reasoning_effort = Some(level);
                    self.post_chat(&body, &root).await?
                }
                Some(errors::ThinkRetry::WithoutThinking) => {
                    body = wire::without_thinking(&request);
                    self.post_chat(&body, &root).await?
                }
                None => return Err(e),
            },
            other => other?,
        };
        // `reasoning_content` is the trace. A model that still wraps it in `<think>` tags inside
        // the text is split the same way Mistral's older checkpoints are.
        let chunks = crate::providers::mistral::separate_think_tags(openai_compat::read_chunks(
            response.bytes_stream(),
            "oMLX",
        ));
        Ok(chunks.boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRole};
    use crate::providers::keys::StaticApiKey;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(server: &MockServer, key: Option<&str>) -> OmlxProvider {
        OmlxProvider::new(
            reqwest::Client::new(),
            Arc::new(StaticApiKey(key.map(String::from))),
            Arc::new(OmlxRequestOptions::new(server.uri())),
        )
    }

    fn request(text: &str) -> LlmRequest {
        LlmRequest {
            model: "qwen3".into(),
            system_instruction: None,
            history: vec![LlmMessage::new(LlmRole::User, text)],
            generation_config: LlmGenerationConfig::default(),
            enable_web_search: None,
        }
    }

    #[tokio::test]
    async fn models_come_from_the_openai_list_and_skip_embeddings() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    {"id": "qwen3", "model_type": "llm", "max_model_len": 32768},
                    {"id": "bge", "model_type": "embedding"}
                ]
            })))
            .mount(&server)
            .await;

        let p = provider(&server, None);
        let models = p.list_models().await.unwrap();
        assert_eq!(models, vec!["qwen3".to_string()]);
        assert_eq!(p.context_window("qwen3").await, Some(32768));
        assert!(server.received_requests().await.unwrap()[0]
            .headers
            .get("authorization")
            .is_none());
    }

    #[tokio::test]
    async fn a_saved_key_is_sent_and_reasoning_lands_in_the_thought_channel() {
        let server = MockServer::start().await;
        let sse = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"planning\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer local-key"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;

        let stream = provider(&server, Some("local-key"))
            .stream_completion(request("Hello"))
            .await
            .unwrap();
        let chunks: Vec<_> = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(chunks[0].thought_delta_text, "planning");
        assert_eq!(chunks[1].delta_text, "Hi");
        let body =
            String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(body.contains("\"reasoning_effort\":\"high\""), "{body}");
        assert!(body.contains("\"enable_thinking\":true"), "{body}");
    }

    #[tokio::test]
    async fn a_template_that_rejects_high_is_retried_at_the_level_it_names() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_string_contains("\"reasoning_effort\":\"high\""))
            .respond_with(ResponseTemplate::new(400).set_body_raw(
                r#"{"detail":"Unexpected reasoning effort high. Supported types are xhigh (default), medium, and low."}"#,
                "application/json",
            ))
            .with_priority(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                "text/event-stream",
            ))
            .with_priority(2)
            .expect(1)
            .mount(&server)
            .await;

        let stream = provider(&server, None)
            .stream_completion(request("Hello"))
            .await
            .unwrap();
        let chunks: Vec<_> = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(chunks[0].delta_text, "Hi");
        let bodies = server.received_requests().await.unwrap();
        let second = String::from_utf8(bodies[1].body.clone()).unwrap();
        assert!(
            second.contains("\"reasoning_effort\":\"xhigh\""),
            "{second}"
        );
        assert!(
            !second.contains("\"reasoning_effort\":\"high\""),
            "{second}"
        );
    }

    #[tokio::test]
    async fn a_model_without_thinking_is_retried_as_a_plain_chat() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"enable_thinking\":true"))
            .respond_with(ResponseTemplate::new(422).set_body_raw(
                r#"{"detail":[{"msg":"enable_thinking is not supported"}]}"#,
                "application/json",
            ))
            .with_priority(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                "text/event-stream",
            ))
            .with_priority(2)
            .expect(1)
            .mount(&server)
            .await;

        let stream = provider(&server, None)
            .stream_completion(request("Hello"))
            .await
            .unwrap();
        let chunks: Vec<_> = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(chunks[0].delta_text, "Hi");
        let second =
            String::from_utf8(server.received_requests().await.unwrap()[1].body.clone()).unwrap();
        assert!(!second.contains("enable_thinking"), "{second}");
        assert!(!second.contains("reasoning_effort"), "{second}");
    }
}
