//! LLM providers. Each maps the provider-agnostic [`LlmRequest`] onto its own wire format and turns
//! the streamed reply back into [`LlmResponseChunk`]s.

pub mod chunk;
pub mod deepseek;
pub mod error;
pub mod gemini;
pub mod http;
pub mod keys;
pub mod ollama;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};

pub use chunk::{LlmGeneratedImage, LlmResponseChunk};
pub use error::{ProviderError, ProviderErrorKind};
pub use keys::{ApiKeyProvider, SecretStore};

use crate::llm::LlmRequest;
use crate::models::LlmUsage;

/// A reply as it arrives. Dropping the stream cancels the underlying HTTP request.
pub type ChunkStream = Pin<Box<dyn Stream<Item = Result<LlmResponseChunk, ProviderError>> + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn provider_id(&self) -> &str;

    /// Live model discovery from the provider's API. Callers needing an offline/pre-refresh list
    /// must cache a prior successful result themselves.
    async fn list_models(&self) -> Result<Vec<String>, ProviderError>;

    /// Starts a streamed completion. Auth and HTTP-status failures are returned here, before any
    /// chunk; failures mid-stream arrive as `Err` items.
    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError>;

    /// Non-streaming convenience: drains the stream into one aggregated final chunk.
    async fn get_completion(&self, request: LlmRequest) -> Result<LlmResponseChunk, ProviderError> {
        let mut stream = self.stream_completion(request).await?;

        let mut text = String::new();
        let mut thought = String::new();
        let mut usage: Option<LlmUsage> = None;
        let mut finish_reason: Option<String> = None;
        let mut images: Vec<LlmGeneratedImage> = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            text.push_str(&chunk.delta_text);
            thought.push_str(&chunk.thought_delta_text);
            if chunk.usage.is_some() {
                usage = chunk.usage;
            }
            if chunk.finish_reason.is_some() {
                finish_reason = chunk.finish_reason;
            }
            images.extend(chunk.images.into_iter().flatten());
        }

        Ok(LlmResponseChunk {
            delta_text: text,
            is_final: true,
            usage,
            finish_reason,
            thought_delta_text: thought,
            images: if images.is_empty() { None } else { Some(images) },
        })
    }
}

/// Resolves configured providers by id, so callers that only know which provider a conversation is
/// pinned to (a plain string) don't depend on every concrete provider.
pub struct LlmProviderRegistry {
    providers: Vec<Arc<dyn LlmProvider>>,
}

impl LlmProviderRegistry {
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Self {
        Self { providers }
    }

    /// All known provider ids, in registration order.
    pub fn provider_ids(&self) -> Vec<String> {
        self.providers.iter().map(|p| p.provider_id().to_string()).collect()
    }

    pub fn get(&self, provider_id: &str) -> Result<Arc<dyn LlmProvider>, ProviderError> {
        self.providers
            .iter()
            .find(|p| p.provider_id().eq_ignore_ascii_case(provider_id))
            .cloned()
            .ok_or_else(|| ProviderError::invalid_request(format!("Unknown provider '{provider_id}'.")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(&'static str);

    #[async_trait]
    impl LlmProvider for Stub {
        fn provider_id(&self) -> &str {
            self.0
        }
        async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
            Ok(vec![])
        }
        async fn stream_completion(&self, _: LlmRequest) -> Result<ChunkStream, ProviderError> {
            let chunks = vec![
                Ok(LlmResponseChunk { delta_text: "Hel".into(), thought_delta_text: "hm".into(), ..Default::default() }),
                Ok(LlmResponseChunk { delta_text: "lo".into(), thought_delta_text: "m".into(), ..Default::default() }),
                Ok(LlmResponseChunk {
                    is_final: true,
                    finish_reason: Some("stop".into()),
                    usage: Some(LlmUsage { prompt_tokens: Some(1), completion_tokens: Some(2), total_tokens: Some(3) }),
                    ..Default::default()
                }),
            ];
            Ok(Box::pin(futures::stream::iter(chunks)))
        }
    }

    fn request() -> LlmRequest {
        LlmRequest {
            model: "m".into(),
            system_instruction: None,
            history: vec![],
            generation_config: Default::default(),
            enable_web_search: None,
        }
    }

    #[tokio::test]
    async fn get_completion_aggregates_text_thoughts_usage_and_finish_reason() {
        let result = Stub("x").get_completion(request()).await.unwrap();
        assert_eq!(result.delta_text, "Hello");
        assert_eq!(result.thought_delta_text, "hmm");
        assert!(result.is_final);
        assert_eq!(result.finish_reason.as_deref(), Some("stop"));
        assert_eq!(result.usage.unwrap().total_tokens, Some(3));
        assert!(result.images.is_none());
    }

    #[test]
    fn registry_resolves_ids_case_insensitively_and_rejects_unknown() {
        let registry = LlmProviderRegistry::new(vec![Arc::new(Stub("gemini")), Arc::new(Stub("ollama"))]);
        assert_eq!(registry.provider_ids(), ["gemini", "ollama"]);
        assert_eq!(registry.get("GEMINI").unwrap().provider_id(), "gemini");
        let err = registry.get("nope").err().unwrap();
        assert_eq!(err.kind, ProviderErrorKind::InvalidRequest);
        assert!(err.message.contains("nope"));
    }
}
