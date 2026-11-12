//! Mistral (`api.mistral.ai`), an OpenAI-compatible API authenticated with a bearer API key. The key
//! is either pasted in or obtained through Mistral's browser sign-in ([`signin`]), which hands back
//! an ordinary API key for the signed-in AI Studio account.

mod errors;
pub mod signin;
mod think;
pub mod wire;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

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
    /// Context lengths from the last model listing, by model id.
    context_windows: Mutex<HashMap<String, i32>>,
    /// Models that refused `reasoning_effort`, so later requests leave it out from the start.
    no_reasoning: Mutex<HashSet<String>>,
    /// Magistral shape that actually got an answer. The next call tries it first.
    shapes: Mutex<HashMap<String, wire::RequestShape>>,
}

impl MistralProvider {
    pub fn new(client: reqwest::Client, api_keys: Arc<dyn ApiKeyProvider>) -> Self {
        Self {
            client,
            api_keys,
            base_url: DEFAULT_BASE_URL.to_string(),
            context_windows: Mutex::default(),
            no_reasoning: Mutex::default(),
            shapes: Mutex::default(),
        }
    }

    async fn post_completion(
        &self,
        key: &str,
        body: &wire::MistralChatCompletionRequest,
    ) -> Result<reqwest::Response, (ProviderError, String)> {
        let json = serde_json::to_string(body).map_err(|e| {
            (
                ProviderError::unknown(format!("Failed to serialize Mistral request: {e}")),
                String::new(),
            )
        })?;
        let http_request = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(key)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .body(json);

        let response = match send(http_request, "Mistral").await {
            Ok(response) => response,
            Err(error) => return Err((error, String::new())),
        };
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let raw = response.text().await.unwrap_or_default();
        Err((errors::to_provider_error(status, &raw), raw))
    }

    async fn fetch_models(&self) -> Result<Vec<wire::MistralModelInfo>, ProviderError> {
        let key = self.require_key()?;
        let request = self
            .client
            .get(format!("{}/models", self.base_url))
            .bearer_auth(key);

        let response = send(request, "Mistral").await?;
        let response = ensure_success(response, errors::to_provider_error).await?;
        let body = response
            .text()
            .await
            .map_err(|e| transport_error("Mistral", &e))?;

        let parsed: wire::MistralListModelsResponse = serde_json::from_str(&body).map_err(|e| {
            ProviderError::unknown(format!("Failed to parse Mistral model list: {e}"))
        })?;
        let models = parsed.data.unwrap_or_default();
        let mut windows = self.context_windows.lock().unwrap();
        for model in &models {
            if let Some(length) = model.max_context_length.filter(|l| *l > 0) {
                windows.insert(model.id.clone(), length);
            }
        }
        Ok(models)
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
            .ok_or_else(|| {
                ProviderError::auth(
                    "No Mistral API key configured - paste one or sign in in Settings → Mistral.",
                )
                .user_facing()
            })
    }
}

#[async_trait]
impl LlmProvider for MistralProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let models = self.fetch_models().await?;
        let mut ids: Vec<String> = models
            .into_iter()
            .filter(|m| m.can_chat())
            .map(|m| m.id)
            .collect();
        // The listing repeats a model under its dated id and its "-latest" alias.
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    async fn context_window(&self, model: &str) -> Option<i32> {
        if let Some(length) = self.context_windows.lock().unwrap().get(model) {
            return Some(*length);
        }
        self.fetch_models().await.ok()?;
        self.context_windows.lock().unwrap().get(model).copied()
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        let key = self.require_key()?;
        let mut attempts = self.attempts_for(&request);
        let mut failures: Vec<AttemptFailure> = Vec::new();
        let mut index = 0;
        while index < attempts.len() {
            let attempt = attempts[index].clone();
            match self.post_completion(&key, &attempt.body).await {
                Ok(response) => {
                    self.note_success(&request.model, attempt.shape);
                    if !failures.is_empty() {
                        log_mistral_debug(&format!(
                            "{} answered with \"{}\" after {} refused shape(s).\n{}",
                            request.model,
                            wire::shape_label(attempt.shape),
                            failures.len(),
                            format_failures(&failures),
                        ));
                    }
                    return Ok(think::separate_think_tags(openai_compat::read_chunks(
                        response.bytes_stream(),
                        "Mistral",
                    ))
                    .boxed());
                }
                Err((error, raw)) => {
                    // Which models take `reasoning_effort` (and which levels) keeps changing, and an
                    // alias like "-latest" moves between them: when Mistral refuses the field, ask
                    // again without and remember that. A refusal that names other accepted levels
                    // (Large 4 rejecting "none") is only this call — remembering it would turn
                    // reasoning off for the rest of the session.
                    let retry_plain = attempt.body.reasoning_effort.is_some()
                        && wire::is_reasoning_rejection(&error);
                    if retry_plain && !wire::rejection_names_accepted_levels(&error.message) {
                        self.no_reasoning
                            .lock()
                            .unwrap()
                            .insert(request.model.clone());
                    }
                    if retry_plain {
                        let plain = Attempt {
                            shape: wire::RequestShape::Plain,
                            body: wire::to_mistral_request_without_reasoning(&request),
                        };
                        if !attempts.iter().any(|existing| existing.body == plain.body) {
                            attempts.push(plain);
                        }
                    }
                    // Magistral's "service not available" does not say which field it rejected, so
                    // try the native shape and then a plain chat before giving up, and keep every
                    // status and body for the chat error.
                    let keep_going = retry_plain
                        || (wire::is_magistral(&request.model) && worth_probing(&error));
                    failures.push(AttemptFailure {
                        shape: attempt.shape,
                        sent: attempt.body,
                        error,
                        raw,
                    });
                    if !keep_going {
                        break;
                    }
                }
            }
            index += 1;
        }
        Err(failure_error(&request.model, &failures))
    }
}

#[derive(Clone)]
struct Attempt {
    shape: wire::RequestShape,
    body: wire::MistralChatCompletionRequest,
}

struct AttemptFailure {
    shape: wire::RequestShape,
    sent: wire::MistralChatCompletionRequest,
    error: ProviderError,
    raw: String,
}

impl MistralProvider {
    fn attempts_for(&self, request: &LlmRequest) -> Vec<Attempt> {
        let one = |shape| Attempt {
            shape,
            body: wire::to_mistral_request_shaped(request, shape),
        };
        if wire::is_magistral(&request.model) {
            let mut shapes = vec![
                wire::RequestShape::Configured,
                wire::RequestShape::Native,
                wire::RequestShape::Plain,
            ];
            if let Some(saved) = self.shapes.lock().unwrap().get(&request.model).copied() {
                shapes.retain(|shape| *shape != saved);
                shapes.insert(0, saved);
            }
            return dedup_attempts(shapes.into_iter().map(one).collect());
        }
        let shape = if self.no_reasoning.lock().unwrap().contains(&request.model) {
            wire::RequestShape::Plain
        } else {
            wire::RequestShape::Configured
        };
        vec![one(shape)]
    }

    fn note_success(&self, model: &str, shape: wire::RequestShape) {
        if !wire::is_magistral(model) {
            return;
        }
        let mut shapes = self.shapes.lock().unwrap();
        if shape == wire::RequestShape::Configured {
            shapes.remove(model);
        } else {
            shapes.insert(model.to_string(), shape);
        }
    }
}

fn dedup_attempts(attempts: Vec<Attempt>) -> Vec<Attempt> {
    let mut kept = Vec::new();
    for attempt in attempts {
        if kept
            .iter()
            .any(|existing: &Attempt| existing.body == attempt.body)
        {
            continue;
        }
        kept.push(attempt);
    }
    kept
}

/// Auth, rate limit, and transport failures will not change because the JSON shape changed.
fn worth_probing(error: &ProviderError) -> bool {
    matches!(
        error.kind,
        super::error::ProviderErrorKind::InvalidRequest | super::error::ProviderErrorKind::Unknown
    )
}

fn failure_error(model: &str, failures: &[AttemptFailure]) -> ProviderError {
    let Some(last) = failures.last() else {
        return ProviderError::unknown(format!("Mistral returned no response for {model}."));
    };
    let detailed = failures.len() > 1
        || failures.iter().any(|failure| {
            matches!(failure.error.http_status, Some(500..=599)) || {
                let message = failure.error.message.to_ascii_lowercase();
                message.contains("not available") || message.contains("unavailable")
            }
        });
    if !detailed {
        return last.error.clone();
    }
    let mut lines = Vec::new();
    if failures.len() == 1 {
        lines.push(format!("Mistral rejected {model}."));
    } else {
        lines.push(format!(
            "Mistral rejected {model}. Tried {} request shapes:",
            failures.len()
        ));
    }
    lines.extend(format_failures(failures).lines().map(str::to_string));
    log_mistral_debug(&lines.join("\n"));
    match last.error.http_status {
        Some(status) => ProviderError::with_status(lines.join("\n"), last.error.kind, status),
        None => ProviderError::new(lines.join("\n"), last.error.kind),
    }
}

fn format_failures(failures: &[AttemptFailure]) -> String {
    let mut lines = Vec::new();
    for (i, failure) in failures.iter().enumerate() {
        let status = failure
            .error
            .http_status
            .map(|code| code.to_string())
            .unwrap_or_else(|| "-".into());
        let prefix = if failures.len() == 1 {
            String::new()
        } else {
            format!("{}. ", i + 1)
        };
        lines.push(format!(
            "{prefix}{} → HTTP {status}: {}",
            wire::shape_label(failure.shape),
            failure.error.message
        ));
        lines.push(format!("sent: {}", wire::describe_request(&failure.sent)));
        let raw = one_line(&failure.raw, 500);
        if !raw.is_empty() && raw != failure.error.message {
            lines.push(format!("body: {raw}"));
        }
    }
    lines.join("\n")
}

fn one_line(text: &str, max_chars: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = flat.chars().count();
    if count <= max_chars {
        flat
    } else {
        format!("{}…", flat.chars().take(max_chars).collect::<String>())
    }
}

/// Appends a refusal report next to the database. Tests skip the file; the chat error already
/// carries the same text when every shape fails.
fn log_mistral_debug(text: &str) {
    eprintln!("Mistral debug:\n{text}");
    #[cfg(not(test))]
    {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let path = std::path::PathBuf::from(home)
            .join("Library/Application Support/me.woelki.localtavern/mistral-debug.log");
        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let mut file = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) => {
                eprintln!(
                    "Mistral debug log {} could not be opened: {error}",
                    path.display()
                );
                return;
            }
        };
        use std::io::Write;
        let _ = writeln!(file, "[{stamp}]\n{text}\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRole};
    use crate::providers::error::ProviderErrorKind;
    use crate::providers::keys::StaticApiKey;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(server: &MockServer, key: Option<&str>) -> MistralProvider {
        MistralProvider::new(
            reqwest::Client::new(),
            Arc::new(StaticApiKey(key.map(String::from))),
        )
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

    async fn collect(
        p: &MistralProvider,
        text: &str,
    ) -> Result<Vec<crate::providers::LlmResponseChunk>, ProviderError> {
        let stream = p
            .stream_completion(request("mistral-large-latest", text))
            .await?;
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

        let chunks = collect(&provider(&server, Some("key-1")), "Hello")
            .await
            .unwrap();

        assert_eq!(chunks[0].delta_text, "Hi");
        let last = chunks.last().unwrap();
        assert_eq!(last.finish_reason.as_deref(), Some("stop"));
        assert_eq!(last.usage.as_ref().unwrap().total_tokens, Some(4));
        let body =
            String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(
            body.contains("\"model\":\"mistral-large-latest\""),
            "{body}"
        );
    }

    #[tokio::test]
    async fn missing_key_is_a_user_facing_auth_error_without_any_http_call() {
        let server = MockServer::start().await;

        let err = collect(&provider(&server, None), "Hello")
            .await
            .unwrap_err();

        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(err.message_is_user_facing);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refused_reasoning_effort_is_retried_without_it_and_remembered() {
        let server = MockServer::start().await;
        let rejection = r#"{"object":"error","message":"Unsupported reasoning level: high","type":"invalid_request_error"}"#;
        Mock::given(method("POST"))
            .and(body_string_contains("reasoning_effort"))
            .respond_with(ResponseTemplate::new(400).set_body_raw(rejection, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        let ok = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(ok, "text/event-stream"))
            .expect(2)
            .mount(&server)
            .await;
        let p = provider(&server, Some("key"));

        for _ in 0..2 {
            let chunks = p
                .stream_completion(request("mistral-large-latest", "Hello"))
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await;
            assert_eq!(chunks[0].as_ref().unwrap().delta_text, "Hi");
        }
        // One rejected request, one retry, then the second call goes straight out without the field.
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_level_rejection_that_names_accepted_levels_does_not_disable_reasoning() {
        let server = MockServer::start().await;
        // Large 4's answer to `reasoning_effort: "none"`. The model still reasons at low/medium/high.
        let rejection = r#"{"object":"error","message":"reasoning_effort none is not supported for this model, supported values: ['high', 'low', 'medium']","type":"invalid_request_error"}"#;
        Mock::given(method("POST"))
            .and(body_string_contains("reasoning_effort"))
            .respond_with(ResponseTemplate::new(400).set_body_raw(rejection, "application/json"))
            .expect(2)
            .mount(&server)
            .await;
        let ok = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(ok, "text/event-stream"))
            .expect(2)
            .mount(&server)
            .await;
        let p = provider(&server, Some("key"));

        for _ in 0..2 {
            let chunks = p
                .stream_completion(request("mistral-large-latest", "Hello"))
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await;
            assert_eq!(chunks[0].as_ref().unwrap().delta_text, "Hi");
        }
    }

    #[tokio::test]
    async fn think_tags_in_the_text_channel_are_shown_as_reasoning() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi <thi\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"nk>because</think> there\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .mount(&server)
            .await;

        let chunks = collect(&provider(&server, Some("key")), "Hello")
            .await
            .unwrap();
        let text: String = chunks.iter().map(|c| c.delta_text.as_str()).collect();
        let thought: String = chunks
            .iter()
            .map(|c| c.thought_delta_text.as_str())
            .collect();
        assert_eq!(text, "Hi  there");
        assert_eq!(thought, "because");
    }

    #[tokio::test]
    async fn error_responses_use_mistrals_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_raw(
                r#"{"object":"error","message":"Rate limit exceeded","type":"rate_limit"}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let err = collect(&provider(&server, Some("key")), "Hello")
            .await
            .unwrap_err();

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
    async fn context_window_comes_from_the_model_listing_and_is_cached() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"data":[{"id":"mistral-small-latest","max_context_length":131072},{"id":"no-length"}]}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let p = provider(&server, Some("key"));

        assert_eq!(p.context_window("mistral-small-latest").await, Some(131072));
        assert_eq!(p.context_window("mistral-small-latest").await, Some(131072));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn list_models_without_a_key_makes_no_http_call() {
        let server = MockServer::start().await;
        let err = provider(&server, None).list_models().await.unwrap_err();
        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    fn magistral_request() -> LlmRequest {
        let mut turn = LlmMessage::new(LlmRole::Model, "Hello");
        turn.reasoning = Some("secret-thought".into());
        LlmRequest {
            model: "magistral-medium-latest".into(),
            system_instruction: Some("secret-system-prompt".into()),
            history: vec![
                LlmMessage::new(LlmRole::User, "secret-prompt-text"),
                turn,
                LlmMessage::new(LlmRole::User, "again"),
            ],
            generation_config: LlmGenerationConfig::default(),
            enable_web_search: None,
        }
    }

    fn sse_ok() -> &'static str {
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
    }

    #[tokio::test]
    async fn magistral_unavailable_tries_the_other_shapes_and_reports_each_refusal() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(503)
                    .set_body_raw(r#"{"message":"plain refused"}"#, "application/json"),
            )
            .with_priority(3)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("thinking"))
            .respond_with(ResponseTemplate::new(503).set_body_raw(
                r#"{"message":"thinking chunks refused"}"#,
                "application/json",
            ))
            .with_priority(2)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("reasoning_effort"))
            .respond_with(
                ResponseTemplate::new(503)
                    .set_body_raw(r#"{"message":"Service not available"}"#, "application/json"),
            )
            .with_priority(1)
            .expect(1)
            .mount(&server)
            .await;
        let p = provider(&server, Some("key"));

        let err = match p.stream_completion(magistral_request()).await {
            Ok(_) => panic!("expected Magistral to refuse every shape"),
            Err(err) => err,
        };

        assert_eq!(err.kind, ProviderErrorKind::Unknown);
        assert_eq!(err.http_status, Some(503));
        assert!(
            err.message.contains("Tried 3 request shapes"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("Service not available"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("thinking chunks refused"),
            "{}",
            err.message
        );
        assert!(err.message.contains("plain refused"), "{}", err.message);
        assert!(
            err.message.contains("reasoning_effort=high"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("reasoning_effort=omitted"),
            "{}",
            err.message
        );
        assert!(
            !err.message.contains("secret-prompt-text"),
            "{}",
            err.message
        );
        assert!(!err.message.contains("secret-thought"), "{}", err.message);
        assert!(
            !err.message.contains("secret-system-prompt"),
            "{}",
            err.message
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn magistral_remembers_the_shape_that_answered() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("thinking"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse_ok(), "text/event-stream"))
            .with_priority(2)
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("reasoning_effort"))
            .respond_with(
                ResponseTemplate::new(503)
                    .set_body_raw(r#"{"message":"Service not available"}"#, "application/json"),
            )
            .with_priority(1)
            .expect(1)
            .mount(&server)
            .await;
        let p = provider(&server, Some("key"));

        for _ in 0..2 {
            let chunks = p
                .stream_completion(magistral_request())
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await;
            assert_eq!(chunks[0].as_ref().unwrap().delta_text, "Hi");
        }

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        let last = String::from_utf8(requests[2].body.clone()).unwrap();
        assert!(!last.contains("reasoning_effort"), "{last}");
        assert!(last.contains("thinking"), "{last}");
    }

    #[tokio::test]
    async fn magistral_auth_failure_is_not_probed_again() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_raw(r#"{"message":"Unauthorized"}"#, "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let err = match provider(&server, Some("key"))
            .stream_completion(magistral_request())
            .await
        {
            Ok(_) => panic!("expected an auth error"),
            Err(err) => err,
        };

        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert_eq!(err.message, "Unauthorized");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
