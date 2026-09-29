//! "Sign in with a subscription": OAuth 2.0 authorization-code + PKCE for a public client, with a
//! short-lived loopback HTTP listener receiving the browser redirect - the flow the vendors' own
//! CLIs use to let a chat subscription authorize API calls - and the persistent session that keeps
//! the resulting tokens fresh.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::engine::general_purpose::{URL_SAFE_NO_PAD, STANDARD_NO_PAD};
use base64::Engine;
use chrono::{DateTime, Utc};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use super::auth::AccessTokenSource;
use super::error::{ProviderError, ProviderErrorKind};
use super::keys::SecretStore;

const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// A browser that connects and then sends nothing must not stall the sign-in.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Refresh this long before the access token actually expires.
const REFRESH_BUFFER: Duration = Duration::from_secs(2 * 60);

/// Everything that differs between "sign in with a subscription" providers. The redirect URI is
/// registered server-side for the (borrowed) public client id, so host, path and the set of ports
/// must match exactly - the first free port in `callback_ports` is used.
#[derive(Debug, Clone)]
pub struct OAuthClientConfig {
    /// User-facing name, e.g. "SuperGrok" - used in messages.
    pub display_name: String,
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scope: String,
    /// Host written into the redirect URI; the listener itself always binds IPv4 loopback.
    pub redirect_host: String,
    pub callback_ports: Vec<u16>,
    pub callback_path: String,
    /// Provider-specific query parameters appended to the authorize URL.
    pub extra_authorize_params: Vec<(String, String)>,
    /// Also send the state value as an OIDC nonce.
    pub send_nonce: bool,
}

/// Result of a token-endpoint call. `refresh_token` is `None` when the server didn't rotate it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub email: Option<String>,
    pub account_id: Option<String>,
}

/// Why a refresh failed: `fatal` means the grant is dead (revoked/expired) and the user must sign
/// in again; otherwise it's worth retrying later.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct OAuthRefreshError {
    pub message: String,
    pub fatal: bool,
}

/// Characters `Uri.EscapeDataString` leaves alone.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

fn form_body(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", utf8_percent_encode(k, UNRESERVED), utf8_percent_encode(v, UNRESERVED)))
        .collect::<Vec<_>>()
        .join("&")
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the operating system's random number generator is available");
    bytes
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn build_authorize_url(config: &OAuthClientConfig, redirect_uri: &str, verifier: &str, state: &str) -> String {
    let challenge = pkce_challenge(verifier);
    let mut pairs: Vec<(&str, &str)> = vec![
        ("client_id", &config.client_id),
        ("redirect_uri", redirect_uri),
        ("scope", &config.scope),
        ("response_type", "code"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ];
    if config.send_nonce {
        pairs.push(("nonce", state));
    }
    pairs.extend(config.extra_authorize_params.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    format!("{}?{}", config.authorize_url, form_body(&pairs))
}

/// Reads the unverified payload of a JWT for display/routing values only (never for trust): the
/// account email and - for OpenAI logins - the ChatGPT account id, which appears in one of three
/// places depending on token vintage.
pub fn extract_claims(jwt: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(payload) = jwt.and_then(|j| j.split('.').nth(1)) else {
        return (None, None);
    };
    let payload = payload.trim_end_matches('=');
    let Some(bytes) = URL_SAFE_NO_PAD.decode(payload).ok().or_else(|| STANDARD_NO_PAD.decode(payload).ok()) else {
        return (None, None);
    };
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (None, None);
    };

    let string_at = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str()).map(str::to_string);
    let email = string_at(root.get("email"));
    let account_id = string_at(root.get("chatgpt_account_id"))
        .or_else(|| string_at(root.get("https://api.openai.com/auth").and_then(|a| a.get("chatgpt_account_id"))))
        .or_else(|| string_at(root.get("organizations").and_then(|o| o.get(0)).and_then(|o| o.get("id"))));
    (email, account_id)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    #[serde(default)]
    expires_in: i64,
    email: Option<String>,
}

fn parse_token_response(json: &str) -> Option<OAuthTokens> {
    let parsed: TokenResponse = serde_json::from_str(json).ok()?;
    let access_token = parsed.access_token.filter(|t| !t.is_empty())?;
    let (email, account_id) = extract_claims(parsed.id_token.as_deref().or(Some(&access_token)));
    Some(OAuthTokens {
        expires_at: Utc::now() + chrono::Duration::seconds(parsed.expires_in),
        refresh_token: parsed.refresh_token.filter(|t| !t.is_empty()),
        email: email.or(parsed.email),
        account_id,
        access_token,
    })
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= 240 {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(240).collect::<String>())
    }
}

pub struct OAuthPkceClient {
    http: reqwest::Client,
    config: OAuthClientConfig,
}

/// What the browser's redirect carried.
#[derive(Debug, Default, PartialEq, Eq)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

impl OAuthPkceClient {
    pub fn new(http: reqwest::Client, config: OAuthClientConfig) -> Self {
        Self { http, config }
    }

    pub fn config(&self) -> &OAuthClientConfig {
        &self.config
    }

    /// Runs the full browser sign-in: starts the loopback listener, hands the authorize URL to
    /// `open_browser`, waits for the redirect, and exchanges the code for tokens. `Ok(None)` means
    /// the user cancelled.
    pub async fn sign_in(
        &self,
        open_browser: impl FnOnce(String) -> Result<(), ProviderError>,
        cancel: &CancellationToken,
    ) -> Result<Option<OAuthTokens>, ProviderError> {
        let verifier = URL_SAFE_NO_PAD.encode(random_bytes::<32>());
        let state = random_bytes::<16>().iter().map(|b| format!("{b:02x}")).collect::<String>();

        // Bind before opening the browser so the redirect can never race the listener.
        let (listener, port) = self.start_listener().await?;
        let redirect_uri = format!("http://{}:{port}{}", self.config.redirect_host, self.config.callback_path);
        let authorize_url = build_authorize_url(&self.config, &redirect_uri, &verifier, &state);

        open_browser(authorize_url)?;

        let callback = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(None),
            _ = tokio::time::sleep(SIGN_IN_TIMEOUT) => {
                return Err(ProviderError::auth("Sign-in timed out - nothing came back from the browser.").user_facing());
            }
            callback = wait_for_callback(&listener, &self.config.callback_path) => callback,
        };
        drop(listener);

        let name = &self.config.display_name;
        if let Some(error) = callback.error {
            return Err(ProviderError::auth(format!("{name} sign-in was not completed: {error}")).user_facing());
        }
        if callback.state.as_deref() != Some(state.as_str()) {
            return Err(ProviderError::auth("Sign-in failed: the redirect's state didn't match (possible stale or forged callback).")
                .user_facing());
        }
        let Some(code) = callback.code.filter(|c| !c.is_empty()) else {
            return Err(ProviderError::auth("Sign-in failed: no authorization code was returned.").user_facing());
        };

        self.exchange_code(&code, &verifier, &redirect_uri).await.map(Some)
    }

    pub async fn exchange_code(&self, code: &str, verifier: &str, redirect_uri: &str) -> Result<OAuthTokens, ProviderError> {
        let body = form_body(&[
            ("grant_type", "authorization_code"),
            ("client_id", &self.config.client_id),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ]);
        let name = &self.config.display_name;
        let response = self.post_token(body).await.map_err(|e| super::http::transport_error(name, &e))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ProviderError::with_status(
                format!("{name} token exchange failed (HTTP {}): {}", status.as_u16(), truncate(&text)),
                ProviderErrorKind::Auth,
                status.as_u16(),
            ));
        }

        let tokens = parse_token_response(&text)
            .ok_or_else(|| ProviderError::unknown(format!("{name} returned an unreadable token response.")))?;
        if tokens.refresh_token.is_none() {
            return Err(ProviderError::auth(format!("{name} didn't return a refresh token, so the session couldn't be kept."))
                .user_facing());
        }
        Ok(tokens)
    }

    /// Fatal for 400/401/403 (dead grant), non-fatal for anything else (network, 5xx).
    pub async fn refresh(&self, refresh_token: &str) -> Result<OAuthTokens, OAuthRefreshError> {
        let body = form_body(&[
            ("grant_type", "refresh_token"),
            ("client_id", &self.config.client_id),
            ("refresh_token", refresh_token),
        ]);
        let name = &self.config.display_name;
        let response = self.post_token(body).await.map_err(|e| OAuthRefreshError {
            message: format!("Network error refreshing the {name} session: {e}"),
            fatal: false,
        })?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let fatal = matches!(status.as_u16(), 400 | 401 | 403);
            return Err(OAuthRefreshError {
                message: format!("{name} token refresh failed (HTTP {}): {}", status.as_u16(), truncate(&text)),
                fatal,
            });
        }
        parse_token_response(&text)
            .ok_or_else(|| OAuthRefreshError { message: format!("{name} returned an unreadable token response."), fatal: false })
    }

    async fn post_token(&self, form: String) -> Result<reqwest::Response, reqwest::Error> {
        self.http
            .post(&self.config.token_url)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .await
    }

    async fn start_listener(&self) -> Result<(TcpListener, u16), ProviderError> {
        let mut last = None;
        for &port in &self.config.callback_ports {
            match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
                Ok(listener) => return Ok((listener, port)),
                Err(e) => last = Some(e),
            }
        }
        let ports = self.config.callback_ports.iter().map(u16::to_string).collect::<Vec<_>>().join("/");
        Err(ProviderError::network(format!(
            "Couldn't listen on 127.0.0.1:{ports} for the sign-in redirect (is another sign-in already running?): {}",
            last.map(|e| e.to_string()).unwrap_or_default()
        ))
        .user_facing())
    }
}

/// Browsers sometimes open speculative connections or ask for `/favicon.ico` first - keeps
/// accepting until the real callback request arrives.
async fn wait_for_callback(listener: &TcpListener, callback_path: &str) -> Callback {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else { continue };
        let Some(request_line) = read_request_line(&mut stream).await else { continue };

        // "GET /callback?code=...&state=... HTTP/1.1"
        let mut parts = request_line.split(' ');
        let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else { continue };
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path != callback_path {
            respond(&mut stream, "404 Not Found", "Not found.").await;
            continue;
        }

        let query = parse_query(query);
        let get = |k: &str| query.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        let mut error = get("error");
        if let (Some(e), Some(description)) = (&error, get("error_description")) {
            error = Some(format!("{e} ({description})"));
        }
        let (code, state) = (get("code"), get("state"));

        let ok = error.is_none() && code.as_deref().is_some_and(|c| !c.is_empty());
        respond(
            &mut stream,
            "200 OK",
            if ok {
                "Signed in to LocalTavern. You can close this tab and return to the app."
            } else {
                "Sign-in didn't complete. You can close this tab and try again from LocalTavern."
            },
        )
        .await;
        return Callback { code, state, error };
    }
}

async fn read_request_line(stream: &mut TcpStream) -> Option<String> {
    let read = async {
        let mut collected = Vec::new();
        let mut buf = [0u8; 1024];
        while collected.len() < 8192 && !collected.contains(&b'\n') {
            let n = stream.read(&mut buf).await.ok()?;
            if n == 0 {
                break;
            }
            collected.extend_from_slice(&buf[..n]);
        }
        let end = collected.iter().position(|&b| b == b'\n').unwrap_or(collected.len());
        Some(String::from_utf8_lossy(&collected[..end]).trim_end_matches('\r').to_string())
    };
    tokio::time::timeout(REQUEST_READ_TIMEOUT, read).await.ok().flatten()
}

async fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let escaped = message.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>LocalTavern</title></head>\
<body style=\"font-family:sans-serif;margin:3em\"><p>{escaped}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    // The browser may have hung up early; the callback was already parsed, so that doesn't matter.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    let decode = |s: &str| percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned();
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(pair), String::new()),
        })
        .collect()
}

// ---------------------------------------------------------------- session

#[derive(Default)]
struct SessionState {
    loaded: bool,
    refresh_token: Option<String>,
    access_token: Option<String>,
    access_token_expires_at: Option<DateTime<Utc>>,
    email: Option<String>,
    account_id: Option<String>,
}

/// Only the refresh token (plus email and account id) is persisted - access tokens are large JWTs
/// that can exceed the OS credential-blob size limit, and re-deriving one on startup is cheap.
#[derive(Serialize, Deserialize)]
struct StoredSession {
    refresh_token: String,
    email: Option<String>,
    account_id: Option<String>,
}

/// One signed-in subscription session: sign-in/out, and handing out a valid access token,
/// refreshing it transparently shortly before it expires.
pub struct OAuthSession {
    client: OAuthPkceClient,
    secrets: Arc<dyn SecretStore>,
    secret_key: String,
    state: Mutex<SessionState>,
    /// Serializes refreshes so concurrent callers share one.
    gate: tokio::sync::Mutex<()>,
}

/// What the settings page shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub signed_in: bool,
    pub email: Option<String>,
}

impl OAuthSession {
    pub fn new(client: OAuthPkceClient, secrets: Arc<dyn SecretStore>, secret_key: &str) -> Self {
        Self { client, secrets, secret_key: secret_key.to_string(), state: Mutex::new(SessionState::default()), gate: Default::default() }
    }

    pub fn display_name(&self) -> &str {
        &self.client.config.display_name
    }

    pub fn status(&self) -> SessionStatus {
        let state = self.loaded_state();
        SessionStatus { signed_in: state.refresh_token.as_deref().is_some_and(|t| !t.is_empty()), email: state.email.clone() }
    }

    /// Locks the state, reading the stored session on first use.
    fn loaded_state(&self) -> std::sync::MutexGuard<'_, SessionState> {
        let mut state = self.state.lock().unwrap();
        if !state.loaded {
            state.loaded = true;
            if let Some(stored) =
                self.secrets.try_get_api_key(&self.secret_key).and_then(|json| serde_json::from_str::<StoredSession>(&json).ok())
            {
                // An unreadable stored session counts as signed out; the next sign-in overwrites it.
                state.refresh_token = Some(stored.refresh_token);
                state.email = stored.email;
                state.account_id = stored.account_id;
            }
        }
        state
    }

    /// Browser sign-in; `Ok(false)` if the user cancelled.
    pub async fn sign_in(
        &self,
        open_browser: impl FnOnce(String) -> Result<(), ProviderError>,
        cancel: &CancellationToken,
    ) -> Result<bool, ProviderError> {
        let Some(tokens) = self.client.sign_in(open_browser, cancel).await? else {
            return Ok(false);
        };
        let _gate = self.gate.lock().await;
        self.apply(tokens, None);
        self.persist().map_err(|e| ProviderError::unknown(format!("Signed in, but the session couldn't be saved: {e}")).user_facing())?;
        Ok(true)
    }

    pub fn sign_out(&self) -> crate::error::Result<()> {
        *self.state.lock().unwrap() = SessionState { loaded: true, ..Default::default() };
        self.secrets.delete_api_key(&self.secret_key)
    }

    fn fresh_access_token(&self) -> Option<String> {
        let state = self.loaded_state();
        let expires_at = state.access_token_expires_at?;
        let buffer = chrono::Duration::from_std(REFRESH_BUFFER).unwrap_or_default();
        state.access_token.clone().filter(|t| !t.is_empty() && Utc::now() + buffer < expires_at)
    }

    fn apply(&self, tokens: OAuthTokens, previous_refresh_token: Option<String>) {
        let mut state = self.state.lock().unwrap();
        state.loaded = true;
        state.access_token = Some(tokens.access_token);
        state.access_token_expires_at = Some(tokens.expires_at);
        // A refresh response may omit the refresh token, meaning "keep using the old one".
        state.refresh_token = tokens.refresh_token.or(previous_refresh_token);
        state.email = tokens.email.or_else(|| state.email.take());
        state.account_id = tokens.account_id.or_else(|| state.account_id.take());
    }

    fn persist(&self) -> crate::error::Result<()> {
        let stored = {
            let state = self.state.lock().unwrap();
            let Some(refresh_token) = state.refresh_token.clone().filter(|t| !t.is_empty()) else {
                return Ok(());
            };
            StoredSession { refresh_token, email: state.email.clone(), account_id: state.account_id.clone() }
        };
        self.secrets.save_api_key(&self.secret_key, &serde_json::to_string(&stored)?)
    }

    fn not_signed_in(&self) -> ProviderError {
        let name = self.display_name();
        ProviderError::auth(format!("Not signed in to {name} - sign in in Settings → {name}.")).user_facing()
    }
}

#[async_trait]
impl AccessTokenSource for OAuthSession {
    async fn access_token(&self) -> Result<String, ProviderError> {
        if let Some(token) = self.fresh_access_token() {
            return Ok(token);
        }

        let _gate = self.gate.lock().await;
        // Another caller may have refreshed while we waited for the gate.
        if let Some(token) = self.fresh_access_token() {
            return Ok(token);
        }

        let refresh_token = self.loaded_state().refresh_token.clone().filter(|t| !t.is_empty()).ok_or_else(|| self.not_signed_in())?;
        let name = self.display_name().to_string();
        match self.client.refresh(&refresh_token).await {
            Ok(tokens) => {
                self.apply(tokens, Some(refresh_token));
                // A failure to save must not fail the request that already has a working token.
                let _ = self.persist();
            }
            Err(e) if e.fatal => {
                let _ = self.sign_out();
                return Err(ProviderError::auth(format!("Your {name} session has expired - sign in again in Settings → {name}."))
                    .user_facing());
            }
            Err(e) => return Err(ProviderError::network(e.message)),
        }
        self.fresh_access_token().ok_or_else(|| self.not_signed_in())
    }

    fn account_id(&self) -> Option<String> {
        self.loaded_state().account_id.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::keys::ApiKeyProvider;
    use crate::secrets::MemorySecretStore;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn config(token_url: &str) -> OAuthClientConfig {
        OAuthClientConfig {
            display_name: "Testly".into(),
            authorize_url: "https://auth.example/authorize".into(),
            token_url: token_url.into(),
            client_id: "client-1".into(),
            scope: "openid offline_access".into(),
            redirect_host: "localhost".into(),
            callback_ports: vec![0],
            callback_path: "/cb".into(),
            extra_authorize_params: vec![("originator".into(), "localtavern".into())],
            send_nonce: false,
        }
    }

    fn jwt(claims: serde_json::Value) -> String {
        format!("h.{}.s", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    fn token_json(access: &str, refresh: Option<&str>, expires_in: i64) -> serde_json::Value {
        let mut body = serde_json::json!({ "access_token": access, "expires_in": expires_in });
        if let Some(r) = refresh {
            body["refresh_token"] = r.into();
        }
        body
    }

    #[test]
    fn pkce_challenge_matches_the_rfc_7636_example() {
        assert_eq!(pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn the_authorize_url_carries_pkce_state_and_extras_escaped() {
        let mut cfg = config("http://unused");
        cfg.send_nonce = true;
        let url = build_authorize_url(&cfg, "http://localhost:1455/cb", "verifier", "st8");
        assert!(url.starts_with("https://auth.example/authorize?client_id=client-1&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fcb&scope=openid%20offline_access&response_type=code&code_challenge="));
        assert!(url.contains(&format!("code_challenge={}", pkce_challenge("verifier"))));
        assert!(url.contains("&code_challenge_method=S256&state=st8&nonce=st8&originator=localtavern"));
    }

    #[test]
    fn claims_are_found_in_all_three_places_and_junk_is_ignored() {
        let direct = jwt(serde_json::json!({ "email": "a@b.c", "chatgpt_account_id": "acc-1" }));
        assert_eq!(extract_claims(Some(&direct)), (Some("a@b.c".into()), Some("acc-1".into())));
        let nested = jwt(serde_json::json!({ "https://api.openai.com/auth": { "chatgpt_account_id": "acc-2" } }));
        assert_eq!(extract_claims(Some(&nested)), (None, Some("acc-2".into())));
        let org = jwt(serde_json::json!({ "organizations": [{ "id": "org-3" }] }));
        assert_eq!(extract_claims(Some(&org)), (None, Some("org-3".into())));

        assert_eq!(extract_claims(None), (None, None));
        assert_eq!(extract_claims(Some("not-a-jwt")), (None, None));
        assert_eq!(extract_claims(Some("a.!!!.c")), (None, None));
        assert_eq!(extract_claims(Some(&format!("a.{}.c", URL_SAFE_NO_PAD.encode("[1]")))), (None, None));
    }

    #[test]
    fn queries_are_decoded() {
        let q = parse_query("code=a%2Bb&state=x+y&error_description=no%20way&flag");
        assert_eq!(q[0], ("code".into(), "a+b".into()));
        assert_eq!(q[1], ("state".into(), "x y".into()));
        assert_eq!(q[2].1, "no way");
        assert_eq!(q[3], ("flag".into(), String::new()));
    }

    #[tokio::test]
    async fn a_code_is_exchanged_for_tokens_with_the_verifier() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .and(body_string_contains("code_verifier=ver"))
            .and(body_string_contains("client_id=client-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "AT", "refresh_token": "RT", "expires_in": 3600,
                "id_token": jwt(serde_json::json!({ "email": "me@x.y", "chatgpt_account_id": "acc" })),
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = OAuthPkceClient::new(reqwest::Client::new(), config(&format!("{}/token", server.uri())));
        let tokens = client.exchange_code("the-code", "ver", "http://localhost/cb").await.unwrap();
        assert_eq!((tokens.access_token.as_str(), tokens.refresh_token.as_deref()), ("AT", Some("RT")));
        assert_eq!((tokens.email.as_deref(), tokens.account_id.as_deref()), (Some("me@x.y"), Some("acc")));
        assert!(tokens.expires_at > Utc::now() + chrono::Duration::minutes(59));
    }

    #[tokio::test]
    async fn exchange_failures_are_auth_errors_and_a_missing_refresh_token_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/bad")).respond_with(ResponseTemplate::new(400).set_body_string("invalid_grant")).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/norefresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_json("AT", None, 60)))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/junk")).respond_with(ResponseTemplate::new(200).set_body_string("<html>")).mount(&server).await;

        let exchange = |p: &str| {
            let client = OAuthPkceClient::new(reqwest::Client::new(), config(&format!("{}/{p}", server.uri())));
            async move { client.exchange_code("c", "v", "r").await.unwrap_err() }
        };
        let bad = exchange("bad").await;
        assert_eq!((bad.kind, bad.http_status), (ProviderErrorKind::Auth, Some(400)));
        assert!(bad.message.contains("invalid_grant"));
        assert!(exchange("norefresh").await.message.contains("refresh token"));
        assert!(exchange("junk").await.message.contains("unreadable"));
    }

    #[tokio::test]
    async fn refresh_failures_are_fatal_only_when_the_grant_is_dead() {
        let server = MockServer::start().await;
        for (route, status) in [("/r400", 400), ("/r401", 401), ("/r403", 403), ("/r500", 500), ("/r429", 429)] {
            Mock::given(method("POST")).and(path(route)).respond_with(ResponseTemplate::new(status)).mount(&server).await;
        }
        let refresh = |route: &str| {
            let client = OAuthPkceClient::new(reqwest::Client::new(), config(&format!("{}{route}", server.uri())));
            async move { client.refresh("RT").await.unwrap_err() }
        };
        assert!(refresh("/r400").await.fatal && refresh("/r401").await.fatal && refresh("/r403").await.fatal);
        assert!(!refresh("/r500").await.fatal && !refresh("/r429").await.fatal);

        // Unreachable server: a network error, worth retrying.
        let dead = OAuthPkceClient::new(reqwest::Client::new(), config("http://127.0.0.1:1/token"));
        assert!(!dead.refresh("RT").await.unwrap_err().fatal);
    }

    /// Plays the browser: follows the authorize URL by calling the loopback redirect with a code.
    fn browser(code_query: &'static str, echo_state: bool) -> impl FnOnce(String) -> Result<(), ProviderError> {
        move |authorize_url: String| {
            let url = reqwest::Url::parse(&authorize_url).unwrap();
            let get = |k: &str| url.query_pairs().find(|(key, _)| key == k).map(|(_, v)| v.into_owned()).unwrap();
            let redirect = get("redirect_uri");
            let state = if echo_state { get("state") } else { "forged".into() };
            let target = format!("{redirect}?{code_query}&state={state}");
            tokio::spawn(async move {
                let _ = reqwest::get(target).await;
            });
            Ok(())
        }
    }

    /// A port that was just free; the tests hand it to the config so parallel tests don't collide.
    fn free_port() -> u16 {
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
    }

    async fn signed_in_client(server: &MockServer) -> OAuthPkceClient {
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("code=granted"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_json("AT", Some("RT"), 3600)))
            .mount(server)
            .await;
        let mut cfg = config(&format!("{}/token", server.uri()));
        cfg.callback_ports = vec![free_port()];
        OAuthPkceClient::new(reqwest::Client::new(), cfg)
    }

    #[tokio::test]
    async fn the_whole_browser_flow_works_against_the_loopback_listener() {
        let server = MockServer::start().await;
        let client = signed_in_client(&server).await;
        let tokens = client.sign_in(browser("code=granted", true), &CancellationToken::new()).await.unwrap().unwrap();
        assert_eq!(tokens.access_token, "AT");
    }

    #[tokio::test]
    async fn a_forged_state_a_denied_login_and_a_missing_code_are_rejected() {
        let server = MockServer::start().await;
        let client = signed_in_client(&server).await;

        let forged = client.sign_in(browser("code=granted", false), &CancellationToken::new()).await.unwrap_err();
        assert!(forged.message.contains("state didn't match"), "{}", forged.message);

        let denied = client
            .sign_in(browser("error=access_denied&error_description=nope", true), &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(denied.message.contains("access_denied (nope)") && denied.message_is_user_facing, "{}", denied.message);

        let no_code = client.sign_in(browser("foo=bar", true), &CancellationToken::new()).await.unwrap_err();
        assert!(no_code.message.contains("no authorization code"));
    }

    #[tokio::test]
    async fn cancelling_stops_the_wait_and_frees_the_port() {
        let server = MockServer::start().await;
        let client = signed_in_client(&server).await;
        let port = client.config.callback_ports[0];
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let result = client
            .sign_in(
                move |_| {
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        trigger.cancel();
                    });
                    Ok(())
                },
                &cancel,
            )
            .await;
        assert_eq!(result.unwrap(), None);
        TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await.expect("the listener was released");
    }

    #[tokio::test]
    async fn sign_in_needs_a_free_port_and_stray_requests_are_ignored() {
        let server = MockServer::start().await;
        let client = signed_in_client(&server).await;
        let port = client.config.callback_ports[0];
        let _taken = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await.unwrap();
        let err = client.sign_in(|_| Ok(()), &CancellationToken::new()).await.unwrap_err();
        assert!(err.message.contains("Couldn't listen"));
        drop(_taken);

        // A favicon request and a silent connection before the real callback don't end the wait.
        let tokens = client
            .sign_in(
                move |authorize_url| {
                    let url = reqwest::Url::parse(&authorize_url).unwrap();
                    let redirect = url.query_pairs().find(|(k, _)| k == "redirect_uri").unwrap().1.into_owned();
                    let state = url.query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
                    tokio::spawn(async move {
                        let _silent = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await;
                        let _ = reqwest::get(format!("http://127.0.0.1:{port}/favicon.ico")).await;
                        let _ = reqwest::get(format!("{redirect}?code=granted&state={state}")).await;
                    });
                    Ok(())
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(tokens.is_some());
    }

    // ---- session ----

    fn session(server: &MockServer, secrets: Arc<MemorySecretStore>) -> OAuthSession {
        OAuthSession::new(OAuthPkceClient::new(reqwest::Client::new(), config(&format!("{}/token", server.uri()))), secrets, "testly")
    }

    #[tokio::test]
    async fn a_stored_session_is_refreshed_once_for_concurrent_callers_and_persists_only_the_refresh_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=RT-old"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": jwt(serde_json::json!({ "email": "me@x.y" })), "refresh_token": "RT-new", "expires_in": 3600,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let secrets = Arc::new(MemorySecretStore::default());
        secrets.save_api_key("testly", r#"{"refresh_token":"RT-old","email":"old@x.y","account_id":"acc-9"}"#).unwrap();
        let session = Arc::new(session(&server, secrets.clone()));
        assert_eq!(session.status(), SessionStatus { signed_in: true, email: Some("old@x.y".into()) });

        let calls: Vec<_> = (0..5)
            .map(|_| {
                let s = session.clone();
                tokio::spawn(async move { s.access_token().await })
            })
            .collect();
        for call in calls {
            assert!(call.await.unwrap().unwrap().starts_with("h."));
        }
        assert_eq!(session.account_id().as_deref(), Some("acc-9"), "kept from the stored session");
        assert_eq!(session.status().email.as_deref(), Some("me@x.y"), "learned from the new token");

        let stored: serde_json::Value = serde_json::from_str(&secrets.try_get_api_key("testly").unwrap()).unwrap();
        assert_eq!(stored["refresh_token"], "RT-new");
        assert!(stored.get("access_token").is_none());
        // The wiremock `expect(1)` verifies on drop that only one refresh went out.
    }

    #[tokio::test]
    async fn a_dead_grant_signs_out_but_a_flaky_server_does_not() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(503)).up_to_n_times(1).mount(&server).await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(400).set_body_string("invalid_grant")).mount(&server).await;

        let secrets = Arc::new(MemorySecretStore::default());
        secrets.save_api_key("testly", r#"{"refresh_token":"RT","email":null,"account_id":null}"#).unwrap();
        let session = session(&server, secrets.clone());

        let flaky = session.access_token().await.unwrap_err();
        assert_eq!(flaky.kind, ProviderErrorKind::Network);
        assert!(session.status().signed_in, "still signed in; try again later");

        let dead = session.access_token().await.unwrap_err();
        assert!(dead.message.contains("session has expired") && dead.message_is_user_facing, "{}", dead.message);
        assert!(!session.status().signed_in);
        assert_eq!(secrets.try_get_api_key("testly"), None);
    }

    #[tokio::test]
    async fn signed_out_sessions_say_so_and_garbage_storage_counts_as_signed_out() {
        let server = MockServer::start().await;
        let secrets = Arc::new(MemorySecretStore::default());
        secrets.save_api_key("testly", "{garbage").unwrap();
        let session = session(&server, secrets);
        assert!(!session.status().signed_in);
        let err = session.access_token().await.unwrap_err();
        assert!(err.message.contains("Not signed in to Testly") && err.message_is_user_facing);
        assert_eq!(session.account_id(), None);
    }

    #[tokio::test]
    async fn signing_in_stores_the_session_and_signing_out_forgets_it() {
        let server = MockServer::start().await;
        let secrets = Arc::new(MemorySecretStore::default());
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "AT", "refresh_token": "RT", "expires_in": 3600, "email": "me@x.y",
            })))
            .mount(&server)
            .await;
        let mut cfg = config(&format!("{}/token", server.uri()));
        cfg.callback_ports = vec![free_port()];
        let session = OAuthSession::new(OAuthPkceClient::new(reqwest::Client::new(), cfg), secrets.clone(), "testly");

        assert!(session.sign_in(browser("code=granted", true), &CancellationToken::new()).await.unwrap());
        assert_eq!(session.status(), SessionStatus { signed_in: true, email: Some("me@x.y".into()) });
        assert_eq!(session.access_token().await.unwrap(), "AT", "no refresh needed right after sign-in");
        assert!(secrets.try_get_api_key("testly").unwrap().contains("\"RT\""));

        session.sign_out().unwrap();
        assert!(!session.status().signed_in);
        assert_eq!(secrets.try_get_api_key("testly"), None);
    }
}
