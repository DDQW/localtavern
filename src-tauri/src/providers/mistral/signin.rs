//! Mistral's browser sign-in: the handshake Mistral's own Vibe CLI uses to log in through your AI
//! Studio account (console.mistral.ai) instead of making you copy an API key by hand.
//!
//! It is not OAuth. The app starts a sign-in with a PKCE challenge, opens the returned page in your
//! browser, polls until you approve it, then trades the one-time exchange token (plus the PKCE
//! verifier) for an ordinary Mistral API key. That key then works exactly like a pasted one.

use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use reqwest::Url;
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::providers::error::{ProviderError, ProviderErrorKind};
use crate::providers::http::send;
use crate::providers::oauth::pkce_challenge;

const CONSOLE_URL: &str = "https://console.mistral.ai";
const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Consecutive failed polls tolerated before giving up (a flaky connection shouldn't abort a sign-in).
const MAX_POLL_FAILURES: u32 = 3;

#[derive(Debug, Deserialize)]
struct StartResponse {
    process_id: String,
    sign_in_url: String,
    poll_url: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct PollResponse {
    status: Option<String>,
    exchange_token: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExchangeResponse {
    api_key: Option<String>,
}

pub struct BrowserSignIn {
    client: reqwest::Client,
    browser_base: String,
    api_base: String,
    poll_interval: Duration,
}

impl BrowserSignIn {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            browser_base: CONSOLE_URL.to_string(),
            api_base: format!("{CONSOLE_URL}/api"),
            poll_interval: POLL_INTERVAL,
        }
    }

    /// Points the sign-in at other servers (tests use a local mock) and polls faster.
    pub fn with_base_urls(mut self, browser_base: &str, api_base: &str, poll_interval: Duration) -> Self {
        self.browser_base = browser_base.trim_end_matches('/').to_string();
        self.api_base = api_base.trim_end_matches('/').to_string();
        self.poll_interval = poll_interval;
        self
    }

    /// Runs the whole sign-in and returns the API key, or `None` if the user cancelled. `open_browser`
    /// is handed the page to show.
    pub async fn sign_in(
        &self,
        open_browser: impl FnOnce(String) -> Result<(), ProviderError>,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        let mut verifier_bytes = [0u8; 64];
        getrandom::fill(&mut verifier_bytes).expect("the operating system's random number generator is available");
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

        let start = self.start(&pkce_challenge(&verifier)).await?;
        let expires_at = DateTime::parse_from_rfc3339(&start.expires_at)
            .map_err(|_| failed("Mistral sent an unreadable sign-in expiry."))?
            .with_timezone(&Utc);
        // The server picks these URLs, so only ever follow ones on the origins we asked.
        let sign_in_url = same_origin(&start.sign_in_url, &self.browser_base)?;
        let poll_url = same_origin(&start.poll_url, &self.api_base)?;

        open_browser(sign_in_url)?;

        let Some(exchange_token) = self.wait_for_approval(&poll_url, expires_at, cancel).await? else {
            return Ok(None);
        };
        self.exchange(&start.process_id, &exchange_token, &verifier).await.map(Some)
    }

    async fn start(&self, code_challenge: &str) -> Result<StartResponse, ProviderError> {
        let request = self.post_json(
            format!("{}/vibe/sign-in", self.api_base),
            &json!({ "code_challenge": code_challenge, "code_challenge_method": "S256" }),
        );
        let response = send(request, "Mistral").await?;
        if !response.status().is_success() {
            return Err(failed(format!("Mistral wouldn't start a sign-in (HTTP {}).", response.status().as_u16())));
        }
        read_json(response).await.ok_or_else(|| failed("Mistral sent an unreadable sign-in response."))
    }

    /// `None` when cancelled; the exchange token once the user has approved access.
    async fn wait_for_approval(
        &self,
        poll_url: &str,
        expires_at: DateTime<Utc>,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        let mut failures = 0;
        while Utc::now() < expires_at {
            match self.poll_once(poll_url).await {
                Ok(Poll::Pending) => failures = 0,
                Ok(Poll::Approved(token)) => return Ok(Some(token)),
                Ok(Poll::Stopped(error)) => return Err(error),
                Err(error) => {
                    failures += 1;
                    if failures >= MAX_POLL_FAILURES {
                        return Err(error);
                    }
                }
            }

            let remaining = (expires_at - Utc::now()).to_std().unwrap_or_default();
            tokio::select! {
                _ = cancel.cancelled() => return Ok(None),
                _ = tokio::time::sleep(self.poll_interval.min(remaining)) => {}
            }
        }
        Err(failed("Sign-in timed out - nothing was approved in the browser."))
    }

    async fn poll_once(&self, poll_url: &str) -> Result<Poll, ProviderError> {
        let response = send(self.client.get(poll_url), "Mistral").await?;
        if response.status() == reqwest::StatusCode::GONE {
            return Ok(Poll::Stopped(failed("The sign-in expired. Try again.")));
        }
        if !response.status().is_success() {
            return Err(ProviderError::network(format!("Couldn't check the sign-in status (HTTP {}).", response.status().as_u16())));
        }
        let poll: PollResponse = read_json(response)
            .await
            .ok_or_else(|| ProviderError::network("Couldn't read the sign-in status.".to_string()))?;
        Ok(match poll.status.as_deref() {
            Some("pending") => Poll::Pending,
            Some("completed") => match poll.exchange_token.filter(|t| !t.is_empty()) {
                Some(token) => Poll::Approved(token),
                None => Poll::Stopped(failed("Signed in, but Mistral didn't hand over a key.")),
            },
            Some("expired") => Poll::Stopped(failed("The sign-in expired. Try again.")),
            Some("denied") => Poll::Stopped(failed("Access was denied in the browser.")),
            Some("error") => Poll::Stopped(failed(poll.message.unwrap_or_else(|| "Mistral reported a sign-in error.".into()))),
            _ => Poll::Stopped(failed("Mistral returned an unknown sign-in state.")),
        })
    }

    async fn exchange(&self, process_id: &str, exchange_token: &str, verifier: &str) -> Result<String, ProviderError> {
        let request = self.post_json(
            format!("{}/vibe/sign-in/{process_id}/exchange", self.api_base),
            &json!({ "exchange_token": exchange_token, "code_verifier": verifier }),
        );
        let response = send(request, "Mistral").await?;
        if !response.status().is_success() {
            return Err(failed(format!("Mistral wouldn't issue a key for this sign-in (HTTP {}).", response.status().as_u16())));
        }
        let parsed: ExchangeResponse = read_json(response).await.ok_or_else(|| failed("Mistral sent an unreadable key response."))?;
        parsed.api_key.filter(|k| !k.is_empty()).ok_or_else(|| failed("Mistral didn't return an API key."))
    }
}

impl BrowserSignIn {
    fn post_json(&self, url: String, body: &serde_json::Value) -> reqwest::RequestBuilder {
        self.client.post(url).header("content-type", "application/json").body(body.to_string())
    }
}

async fn read_json<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Option<T> {
    serde_json::from_str(&response.text().await.ok()?).ok()
}

enum Poll {
    Pending,
    Approved(String),
    Stopped(ProviderError),
}

fn failed(message: impl Into<String>) -> ProviderError {
    ProviderError::new(message, ProviderErrorKind::Auth).user_facing()
}

/// Returns `url` unchanged when it is on the same scheme/host/port as `base`.
fn same_origin(url: &str, base: &str) -> Result<String, ProviderError> {
    let origin = |u: &Url| (u.scheme().to_string(), u.host_str().map(str::to_string), u.port_or_known_default());
    match (Url::parse(url), Url::parse(base)) {
        (Ok(candidate), Ok(expected)) if origin(&candidate) == origin(&expected) => Ok(url.to_string()),
        _ => Err(failed("Mistral sent a sign-in address on an unexpected server, so it wasn't opened.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sign_in_for(server: &MockServer) -> BrowserSignIn {
        BrowserSignIn::new(reqwest::Client::new()).with_base_urls(&server.uri(), &format!("{}/api", server.uri()), Duration::from_millis(10))
    }

    fn json_response(status: u16, body: serde_json::Value) -> ResponseTemplate {
        ResponseTemplate::new(status).set_body_json(body)
    }

    async fn mount_start(server: &MockServer, expires_at: &str) {
        let uri = server.uri();
        Mock::given(method("POST"))
            .and(path("/api/vibe/sign-in"))
            .and(body_partial_json(json!({ "code_challenge_method": "S256" })))
            .respond_with(json_response(
                200,
                json!({
                    "process_id": "proc-1",
                    "sign_in_url": format!("{uri}/vibe/approve?process=proc-1"),
                    "poll_url": format!("{uri}/api/vibe/sign-in/proc-1"),
                    "expires_at": expires_at,
                }),
            ))
            .mount(server)
            .await;
    }

    fn in_a_minute() -> String {
        (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339()
    }

    #[tokio::test]
    async fn signs_in_polls_until_approved_and_exchanges_for_a_key() {
        let server = MockServer::start().await;
        mount_start(&server, &in_a_minute()).await;
        Mock::given(method("GET"))
            .and(path("/api/vibe/sign-in/proc-1"))
            .respond_with(json_response(200, json!({ "status": "pending" })))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/vibe/sign-in/proc-1"))
            .respond_with(json_response(200, json!({ "status": "completed", "exchange_token": "xt-9" })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/vibe/sign-in/proc-1/exchange"))
            .and(body_partial_json(json!({ "exchange_token": "xt-9" })))
            .respond_with(json_response(200, json!({ "api_key": "mk-secret" })))
            .expect(1)
            .mount(&server)
            .await;

        let opened = Arc::new(Mutex::new(None));
        let sink = opened.clone();
        let key = sign_in_for(&server)
            .sign_in(
                move |url| {
                    *sink.lock().unwrap() = Some(url);
                    Ok(())
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(key.as_deref(), Some("mk-secret"));
        assert_eq!(opened.lock().unwrap().as_deref(), Some(format!("{}/vibe/approve?process=proc-1", server.uri()).as_str()));
        // The PKCE verifier must pair with the challenge sent at the start.
        let requests = server.received_requests().await.unwrap();
        let start: serde_json::Value = serde_json::from_slice(&requests.iter().find(|r| r.url.path() == "/api/vibe/sign-in").unwrap().body).unwrap();
        let exchange: serde_json::Value = serde_json::from_slice(&requests.iter().find(|r| r.url.path().ends_with("/exchange")).unwrap().body).unwrap();
        assert_eq!(start["code_challenge"], pkce_challenge(exchange["code_verifier"].as_str().unwrap()));
    }

    #[tokio::test]
    async fn cancelling_stops_the_wait_without_an_error() {
        let server = MockServer::start().await;
        mount_start(&server, &in_a_minute()).await;
        Mock::given(method("GET")).respond_with(json_response(200, json!({ "status": "pending" }))).mount(&server).await;
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();

        let key = sign_in_for(&server)
            .sign_in(
                move |_| {
                    trigger.cancel();
                    Ok(())
                },
                &cancel,
            )
            .await
            .unwrap();

        assert_eq!(key, None);
    }

    #[tokio::test]
    async fn a_denied_sign_in_is_a_user_facing_auth_error() {
        let server = MockServer::start().await;
        mount_start(&server, &in_a_minute()).await;
        Mock::given(method("GET")).respond_with(json_response(200, json!({ "status": "denied" }))).mount(&server).await;

        let err = sign_in_for(&server).sign_in(|_| Ok(()), &CancellationToken::new()).await.unwrap_err();

        assert_eq!(err.kind, ProviderErrorKind::Auth);
        assert!(err.message_is_user_facing);
        assert!(err.message.contains("denied"), "{}", err.message);
    }

    #[tokio::test]
    async fn a_gone_poll_means_the_sign_in_expired() {
        let server = MockServer::start().await;
        mount_start(&server, &in_a_minute()).await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(410)).mount(&server).await;

        let err = sign_in_for(&server).sign_in(|_| Ok(()), &CancellationToken::new()).await.unwrap_err();

        assert!(err.message.contains("expired"), "{}", err.message);
    }

    #[tokio::test]
    async fn gives_up_after_repeated_poll_failures_but_survives_a_blip() {
        let server = MockServer::start().await;
        mount_start(&server, &in_a_minute()).await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(500)).mount(&server).await;

        let err = sign_in_for(&server).sign_in(|_| Ok(()), &CancellationToken::new()).await.unwrap_err();

        assert_eq!(err.kind, ProviderErrorKind::Network);
        let polls = server.received_requests().await.unwrap().iter().filter(|r| r.method.as_str() == "GET").count();
        assert_eq!(polls, MAX_POLL_FAILURES as usize);
    }

    #[tokio::test]
    async fn an_expired_process_times_out_without_polling_forever() {
        let server = MockServer::start().await;
        mount_start(&server, &(Utc::now() - chrono::Duration::seconds(5)).to_rfc3339()).await;

        let err = sign_in_for(&server).sign_in(|_| Ok(()), &CancellationToken::new()).await.unwrap_err();

        assert!(err.message.contains("timed out"), "{}", err.message);
    }

    #[tokio::test]
    async fn refuses_to_open_a_sign_in_page_on_another_server() {
        let server = MockServer::start().await;
        let uri = server.uri();
        Mock::given(method("POST"))
            .and(path("/api/vibe/sign-in"))
            .respond_with(json_response(
                200,
                json!({
                    "process_id": "p",
                    "sign_in_url": "https://evil.example/phish",
                    "poll_url": format!("{uri}/api/vibe/sign-in/p"),
                    "expires_at": in_a_minute(),
                }),
            ))
            .mount(&server)
            .await;
        let opened = Arc::new(Mutex::new(false));
        let flag = opened.clone();

        let err = sign_in_for(&server)
            .sign_in(
                move |_| {
                    *flag.lock().unwrap() = true;
                    Ok(())
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();

        assert!(!*opened.lock().unwrap());
        assert!(err.message.contains("unexpected server"), "{}", err.message);
    }

    #[tokio::test]
    async fn a_missing_key_in_the_exchange_is_an_error() {
        let server = MockServer::start().await;
        mount_start(&server, &in_a_minute()).await;
        Mock::given(method("GET"))
            .respond_with(json_response(200, json!({ "status": "completed", "exchange_token": "xt" })))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/api/vibe/sign-in/proc-1/exchange")).respond_with(json_response(200, json!({}))).mount(&server).await;

        let err = sign_in_for(&server).sign_in(|_| Ok(()), &CancellationToken::new()).await.unwrap_err();

        assert!(err.message.contains("API key"), "{}", err.message);
    }
}
