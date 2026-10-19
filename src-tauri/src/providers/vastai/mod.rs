//! A GPU rented on Vast.ai, started when a request needs it and stopped again once the answer is
//! done. A stopped instance only bills for its disk, so a model can live on a big GPU without it
//! costing money while nobody is chatting.
//!
//! The instance is expected to run an Ollama server (Vast's Ollama template, or any image with
//! `ollama serve` on [`VastAiOptions::port`]). Requests are forwarded to [`OllamaProvider`] pointed at
//! wherever the instance currently lives - the public IP and published port change on every start.
//!
//! Every request holds a [`Lease`]. The first lease starts the instance and waits until Ollama
//! answers; when the last lease is dropped (the reply finished, failed or was cancelled) the stop is
//! scheduled after `idle_stop_secs`, and any new request in that window cancels it. The grace period
//! keeps a reply and the follow-up calls the app makes right after it (memory extraction,
//! compaction, picture prompts) from each paying for a restart.

pub mod api;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

pub use api::{InstanceControl, InstanceStatus, VastApi};

use super::error::ProviderError;
use super::keys::ApiKeyProvider;
use super::ollama::{OllamaOptionsSource, OllamaProvider, OllamaRequestOptions};
use super::{ChunkStream, LlmProvider};
use crate::llm::LlmRequest;

pub const PROVIDER_ID: &str = "vastai";

pub const DEFAULT_PORT: u16 = 11434;
pub const DEFAULT_CONTEXT_SIZE: i32 = 8192;
pub const DEFAULT_IDLE_STOP_SECS: u64 = 10;
pub const DEFAULT_READY_TIMEOUT_SECS: u64 = 600;
const STOP_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VastAiOptions {
    /// The instance's ID from the Vast.ai console (the number in its row). 0 = not set up yet.
    pub instance_id: i64,
    /// The container port Ollama listens on.
    pub port: u16,
    /// Use this address instead of the instance's public IP and published port - for a tunnel or a
    /// fixed domain. Empty = look the address up.
    pub base_url: String,
    pub context_size: i32,
    /// How long the instance stays up after the last request finishes. 0 = stop right away.
    pub idle_stop_secs: u64,
    /// How long to wait for a starting instance to answer before giving up.
    pub ready_timeout_secs: u64,
}

impl Default for VastAiOptions {
    fn default() -> Self {
        Self {
            instance_id: 0,
            port: DEFAULT_PORT,
            base_url: String::new(),
            context_size: DEFAULT_CONTEXT_SIZE,
            idle_stop_secs: DEFAULT_IDLE_STOP_SECS,
            ready_timeout_secs: DEFAULT_READY_TIMEOUT_SECS,
        }
    }
}

pub trait VastAiOptionsSource: Send + Sync {
    fn current(&self) -> VastAiOptions;
}

impl VastAiOptionsSource for VastAiOptions {
    fn current(&self) -> VastAiOptions {
        self.clone()
    }
}

/// Hands the inner Ollama provider the address the instance was last found at.
struct LiveOllamaOptions {
    url: Arc<Mutex<String>>,
    options: Arc<dyn VastAiOptionsSource>,
}

impl OllamaOptionsSource for LiveOllamaOptions {
    fn current(&self) -> OllamaRequestOptions {
        OllamaRequestOptions { base_url: self.url.lock().unwrap().clone(), context_size: self.options.current().context_size }
    }
}

#[derive(Default)]
struct LeaseState {
    active: usize,
    /// Bumped whenever the number of leases changes, so a scheduled stop can tell it is out of date.
    generation: u64,
}

struct Shared {
    control: Arc<dyn InstanceControl>,
    client: reqwest::Client,
    options: Arc<dyn VastAiOptionsSource>,
    url: Arc<Mutex<String>>,
    state: Mutex<LeaseState>,
    /// Serialises starting and stopping, so a stop never lands in the middle of a start.
    gate: futures::lock::Mutex<()>,
    poll_interval: Duration,
    /// Whether this app run has started (or used) the instance, i.e. has something to stop on exit.
    touched: AtomicBool,
}

/// One request's claim on the instance. Dropping the last one schedules the stop.
struct Lease {
    shared: Arc<Shared>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let generation = {
            let mut state = self.shared.state.lock().unwrap();
            state.active -= 1;
            state.generation += 1;
            if state.active > 0 {
                return;
            }
            state.generation
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(self.shared.clone().stop_when_idle(generation));
        }
    }
}

impl Shared {
    fn lease(self: &Arc<Self>) -> Lease {
        let mut state = self.state.lock().unwrap();
        state.active += 1;
        state.generation += 1;
        Lease { shared: self.clone() }
    }

    async fn probe(&self, base_url: &str) -> bool {
        let request = self.client.get(format!("{base_url}/api/tags")).timeout(Duration::from_secs(5));
        matches!(request.send().await, Ok(response) if response.status().is_success())
    }

    /// Starts the instance if needed and waits until Ollama answers on it.
    async fn ensure_running(&self) -> Result<(), ProviderError> {
        let options = self.options.current();
        if options.instance_id <= 0 {
            return Err(ProviderError::invalid_request("Enter your Vast.ai instance ID in Settings → Vast.ai.").user_facing());
        }
        let _gate = self.gate.lock().await;
        self.touched.store(true, Ordering::Relaxed);

        let started = Instant::now();
        let timeout = Duration::from_secs(options.ready_timeout_secs.max(1));
        let override_url = options.base_url.trim().trim_end_matches('/').to_string();
        let mut requested = false;
        let mut last = InstanceStatus::default();
        loop {
            last = match self.control.status(options.instance_id).await {
                Ok(status) => status,
                // A rate limit or hiccup mid-wait is not worth abandoning a start over.
                Err(e) if requested && started.elapsed() < timeout && e.http_status != Some(401) && e.http_status != Some(404) => last,
                Err(e) => return Err(e),
            };
            if last.is_running() {
                let base = if override_url.is_empty() {
                    last.endpoint(options.port).map(|endpoint| format!("http://{endpoint}"))
                } else {
                    Some(override_url.clone())
                };
                if let Some(base) = base {
                    if self.probe(&base).await {
                        *self.url.lock().unwrap() = base;
                        return Ok(());
                    }
                }
            } else if !requested {
                self.control.set_running(options.instance_id, true).await?;
                requested = true;
            }

            if started.elapsed() >= timeout {
                let status = last.actual.as_deref().unwrap_or("unknown");
                let detail = last.message.as_deref().map(|m| format!(": {m}")).unwrap_or_default();
                return Err(ProviderError::network(format!(
                    "The Vast.ai instance didn't come up within {} s (status \"{status}\"{detail}). \
                     A stopped instance can fail to start when its GPU was rented out meanwhile - check the Vast.ai console.",
                    timeout.as_secs()
                ))
                .user_facing());
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    async fn stop_when_idle(self: Arc<Self>, generation: u64) {
        let idle = Duration::from_secs(self.options.current().idle_stop_secs);
        if !idle.is_zero() {
            tokio::time::sleep(idle).await;
        }
        let _gate = self.gate.lock().await;
        {
            let state = self.state.lock().unwrap();
            if state.active > 0 || state.generation != generation {
                return;
            }
        }
        self.stop().await;
    }

    /// Stops the instance, retrying - a stop that silently failed keeps the GPU billing.
    async fn stop(&self) {
        let id = self.options.current().instance_id;
        if id <= 0 {
            return;
        }
        for attempt in 1..=STOP_ATTEMPTS {
            match self.control.set_running(id, false).await {
                Ok(()) => return,
                Err(e) if attempt == STOP_ATTEMPTS => {
                    eprintln!("Vast.ai: couldn't stop instance {id} - it may still be running and billing: {}", e.message);
                }
                Err(_) => tokio::time::sleep(self.poll_interval).await,
            }
        }
    }
}

pub struct VastAiProvider {
    shared: Arc<Shared>,
    inner: OllamaProvider,
}

impl VastAiProvider {
    pub fn new(client: reqwest::Client, api_keys: Arc<dyn ApiKeyProvider>, options: Arc<dyn VastAiOptionsSource>) -> Self {
        let control = Arc::new(VastApi::new(client.clone(), api_keys));
        Self::with_control(client, control, options)
    }

    pub fn with_control(client: reqwest::Client, control: Arc<dyn InstanceControl>, options: Arc<dyn VastAiOptionsSource>) -> Self {
        let url = Arc::new(Mutex::new(String::new()));
        let inner = OllamaProvider::new(client.clone(), Arc::new(LiveOllamaOptions { url: url.clone(), options: options.clone() }));
        let shared = Shared {
            control,
            client,
            options,
            url,
            state: Mutex::default(),
            gate: futures::lock::Mutex::new(()),
            poll_interval: Duration::from_secs(5),
            touched: AtomicBool::new(false),
        };
        Self { shared: Arc::new(shared), inner }
    }

    #[cfg(test)]
    fn with_poll_interval(mut self, interval: Duration) -> Self {
        Arc::get_mut(&mut self.shared).expect("not shared yet").poll_interval = interval;
        self
    }

    /// Stops the instance now if this app run used it. Called when the app exits, so a reply
    /// that finished a moment before closing doesn't leave the GPU running.
    pub async fn shutdown(&self) {
        if !self.shared.touched.load(Ordering::Relaxed) {
            return;
        }
        let _gate = self.shared.gate.lock().await;
        self.shared.stop().await;
    }
}

#[async_trait]
impl LlmProvider for VastAiProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    /// Needs the instance up, so refreshing the list starts it (and it stops again afterwards).
    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let _lease = self.shared.lease();
        self.shared.ensure_running().await?;
        self.inner.list_models().await
    }

    async fn context_window(&self, _model: &str) -> Option<i32> {
        Some(self.shared.options.current().context_size)
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        let lease = self.shared.lease();
        self.shared.ensure_running().await?;
        let mut chunks = self.inner.stream_completion(request).await?;
        // The lease rides along with the stream: it ends when the reply does, or when the stream is
        // dropped by a cancel.
        Ok(Box::pin(async_stream::stream! {
            let _lease = lease;
            while let Some(chunk) = chunks.next().await {
                yield chunk;
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRole};
    use serde_json::json;
    use std::collections::VecDeque;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Plays back scripted statuses and records the start/stop requests.
    struct FakeControl {
        statuses: Mutex<VecDeque<InstanceStatus>>,
        last: Mutex<InstanceStatus>,
        calls: Mutex<Vec<bool>>,
        stop_failures: Mutex<usize>,
    }

    impl FakeControl {
        fn new(statuses: Vec<InstanceStatus>) -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(statuses.into()),
                last: Mutex::default(),
                calls: Mutex::default(),
                stop_failures: Mutex::new(0),
            })
        }

        fn calls(&self) -> Vec<bool> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl InstanceControl for FakeControl {
        async fn status(&self, _: i64) -> Result<InstanceStatus, ProviderError> {
            let mut last = self.last.lock().unwrap();
            if let Some(next) = self.statuses.lock().unwrap().pop_front() {
                *last = next;
            }
            Ok(last.clone())
        }

        async fn set_running(&self, _: i64, running: bool) -> Result<(), ProviderError> {
            if !running {
                let mut failures = self.stop_failures.lock().unwrap();
                if *failures > 0 {
                    *failures -= 1;
                    return Err(ProviderError::network("flaky"));
                }
            }
            self.calls.lock().unwrap().push(running);
            Ok(())
        }
    }

    fn status(actual: &str, intended: &str, server: Option<&MockServer>) -> InstanceStatus {
        let ports = match server {
            Some(s) => json!({"11434/tcp": [{"HostIp": "0.0.0.0", "HostPort": s.address().port().to_string()}]}),
            None => json!({}),
        };
        InstanceStatus {
            actual: Some(actual.into()),
            intended: Some(intended.into()),
            public_ip: server.map(|s| s.address().ip().to_string()),
            ports,
            ..Default::default()
        }
    }

    /// An Ollama stand-in that answers the readiness probe and a one-line chat.
    async fn ollama() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/tags")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{"name": "m1"}]}))).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "{\"message\":{\"role\":\"assistant\",\"content\":\"hi\"},\"done\":false}\n{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\"}\n",
            ))
            .mount(&server)
            .await;
        server
    }

    fn options(idle: u64) -> Arc<VastAiOptions> {
        Arc::new(VastAiOptions { instance_id: 7, idle_stop_secs: idle, ready_timeout_secs: 5, ..Default::default() })
    }

    fn provider(control: Arc<FakeControl>, options: Arc<VastAiOptions>) -> VastAiProvider {
        VastAiProvider::with_control(reqwest::Client::new(), control, options).with_poll_interval(Duration::from_millis(10))
    }

    fn request() -> LlmRequest {
        LlmRequest {
            model: "m1".into(),
            system_instruction: None,
            history: vec![LlmMessage::new(LlmRole::User, "hello")],
            generation_config: LlmGenerationConfig::default(),
            enable_web_search: None,
        }
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    #[tokio::test]
    async fn an_answer_starts_a_stopped_instance_and_stops_it_afterwards() {
        let server = ollama().await;
        let control = FakeControl::new(vec![
            status("exited", "stopped", None),
            status("loading", "running", None),
            status("running", "running", Some(&server)),
        ]);
        let vast = provider(control.clone(), options(0));

        let reply = vast.get_completion(request()).await.unwrap();
        assert_eq!(reply.delta_text, "hi");
        settle().await;

        assert_eq!(control.calls(), [true, false], "start once, then stop once the reply is done");
    }

    #[tokio::test]
    async fn an_instance_that_is_already_up_is_not_started_again() {
        let server = ollama().await;
        let control = FakeControl::new(vec![status("running", "running", Some(&server))]);
        let vast = provider(control.clone(), options(0));

        vast.get_completion(request()).await.unwrap();
        settle().await;

        assert_eq!(control.calls(), [false]);
    }

    #[tokio::test]
    async fn a_running_instance_that_is_on_its_way_down_is_started_again() {
        let server = ollama().await;
        let control = FakeControl::new(vec![status("running", "stopped", Some(&server)), status("running", "running", Some(&server))]);
        let vast = provider(control.clone(), options(60));

        vast.get_completion(request()).await.unwrap();

        assert_eq!(control.calls(), [true]);
    }

    #[tokio::test]
    async fn a_second_request_within_the_grace_period_keeps_the_instance_up() {
        let server = ollama().await;
        let control = FakeControl::new(vec![status("running", "running", Some(&server))]);
        let vast = provider(control.clone(), options(1));

        vast.get_completion(request()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        vast.get_completion(request()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(control.calls().is_empty(), "no stop while the grace period keeps being renewed");

        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(control.calls(), [false], "stopped once, after the last reply");
    }

    #[tokio::test]
    async fn dropping_the_stream_early_still_stops_the_instance() {
        let server = ollama().await;
        let control = FakeControl::new(vec![status("running", "running", Some(&server))]);
        let vast = provider(control.clone(), options(0));

        let stream = vast.stream_completion(request()).await.unwrap();
        drop(stream);
        settle().await;

        assert_eq!(control.calls(), [false]);
    }

    #[tokio::test]
    async fn the_instance_stays_up_while_a_reply_is_still_streaming() {
        let server = ollama().await;
        let control = FakeControl::new(vec![status("running", "running", Some(&server))]);
        let vast = provider(control.clone(), options(0));

        let mut stream = vast.stream_completion(request()).await.unwrap();
        settle().await;
        assert!(control.calls().is_empty());
        while stream.next().await.is_some() {}
        drop(stream);
        settle().await;
        assert_eq!(control.calls(), [false]);
    }

    #[tokio::test]
    async fn a_failed_start_reports_the_status_and_still_stops() {
        let control = FakeControl::new(vec![InstanceStatus { actual: Some("exited".into()), intended: Some("stopped".into()), message: Some("no GPU".into()), ..Default::default() }]);
        let opts = Arc::new(VastAiOptions { instance_id: 7, idle_stop_secs: 0, ready_timeout_secs: 1, ..Default::default() });
        let vast = provider(control.clone(), opts);

        let err = vast.get_completion(request()).await.unwrap_err();
        assert!(err.message.contains("exited") && err.message.contains("no GPU"), "{}", err.message);
        settle().await;

        assert_eq!(control.calls(), [true, false]);
    }

    #[tokio::test]
    async fn a_failing_stop_is_retried() {
        let server = ollama().await;
        let control = FakeControl::new(vec![status("running", "running", Some(&server))]);
        *control.stop_failures.lock().unwrap() = 2;
        let vast = provider(control.clone(), options(0));

        vast.get_completion(request()).await.unwrap();
        settle().await;

        assert_eq!(control.calls(), [false]);
    }

    #[tokio::test]
    async fn listing_models_goes_through_the_instance() {
        let server = ollama().await;
        let control = FakeControl::new(vec![status("running", "running", Some(&server))]);
        let vast = provider(control.clone(), options(0));

        assert_eq!(vast.list_models().await.unwrap(), ["m1"]);
        settle().await;
        assert_eq!(control.calls(), [false]);
    }

    #[tokio::test]
    async fn an_unconfigured_instance_asks_for_the_id_without_calling_vast() {
        let control = FakeControl::new(vec![]);
        let vast = provider(control.clone(), Arc::new(VastAiOptions::default()));

        let err = vast.get_completion(request()).await.unwrap_err();
        assert!(err.message.contains("instance ID"));
        settle().await;
        assert!(control.calls().is_empty());
    }

    #[tokio::test]
    async fn shutdown_stops_only_an_instance_this_run_used() {
        let server = ollama().await;
        let control = FakeControl::new(vec![status("running", "running", Some(&server))]);
        let vast = provider(control.clone(), options(60));

        vast.shutdown().await;
        assert!(control.calls().is_empty(), "untouched instance is left alone");

        vast.get_completion(request()).await.unwrap();
        vast.shutdown().await;
        assert_eq!(control.calls(), [false]);
    }

    #[tokio::test]
    async fn the_context_window_is_the_configured_size_and_needs_no_instance() {
        let control = FakeControl::new(vec![]);
        let vast = provider(control.clone(), options(0));
        assert_eq!(vast.context_window("m1").await, Some(DEFAULT_CONTEXT_SIZE));
        assert!(control.calls().is_empty());
    }
}
