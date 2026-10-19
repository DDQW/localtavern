//! The slice of the Vast.ai REST API this provider needs: read an instance, and ask it to start or
//! stop. Kept behind [`InstanceControl`] so the lifecycle logic can be tested with a fake.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::Value;

use crate::providers::error::{ProviderError, ProviderErrorKind};
use crate::providers::http::{describe_status, send};
use crate::providers::keys::ApiKeyProvider;

const BASE_URL: &str = "https://console.vast.ai/api/v0";
const NAME: &str = "Vast.ai";

/// What Vast reports about one instance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstanceStatus {
    /// What the container is doing right now ("running", "loading", "exited", ...).
    pub actual: Option<String>,
    /// What it has been asked to do ("running" or "stopped").
    pub intended: Option<String>,
    pub message: Option<String>,
    pub public_ip: Option<String>,
    /// Docker-style mapping: `{"11434/tcp": [{"HostIp": "0.0.0.0", "HostPort": "40012"}]}`.
    pub ports: Value,
}

impl InstanceStatus {
    pub fn from_json(value: &Value) -> Self {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        Self {
            actual: text("actual_status"),
            intended: text("intended_status"),
            message: text("status_msg"),
            public_ip: text("public_ipaddr"),
            ports: value.get("ports").cloned().unwrap_or(Value::Null),
        }
    }

    /// Up and not on its way down. Vast keeps reporting `actual_status: running` for a while after a
    /// stop request, so the intended state has to agree.
    pub fn is_running(&self) -> bool {
        self.actual.as_deref() == Some("running") && self.intended.as_deref() != Some("stopped")
    }

    /// The `host:port` the container's `container_port` is published on, once the instance has one.
    pub fn endpoint(&self, container_port: u16) -> Option<String> {
        let ip = self.public_ip.as_deref()?;
        let host_port = self
            .ports
            .get(format!("{container_port}/tcp"))?
            .as_array()?
            .iter()
            .find_map(|binding| binding.get("HostPort").and_then(Value::as_str))?;
        Some(format!("{ip}:{host_port}"))
    }
}

#[async_trait]
pub trait InstanceControl: Send + Sync {
    async fn status(&self, instance_id: i64) -> Result<InstanceStatus, ProviderError>;
    /// Asks Vast to start (`true`) or stop (`false`) the instance. Returns once Vast accepted the
    /// request, not once the instance got there.
    async fn set_running(&self, instance_id: i64, running: bool) -> Result<(), ProviderError>;
}

pub struct VastApi {
    client: reqwest::Client,
    api_keys: Arc<dyn ApiKeyProvider>,
    base_url: String,
}

impl VastApi {
    pub fn new(client: reqwest::Client, api_keys: Arc<dyn ApiKeyProvider>) -> Self {
        Self { client, api_keys, base_url: BASE_URL.to_string() }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    fn key(&self) -> Result<String, ProviderError> {
        self.api_keys
            .try_get_api_key(super::PROVIDER_ID)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| ProviderError::auth("No Vast.ai API key configured - paste one in Settings → Vast.ai.").user_facing())
    }

    async fn check(response: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(to_provider_error(status, &body))
    }
}

pub fn to_provider_error(status: StatusCode, body: &str) -> ProviderError {
    let detail = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        ["msg", "error"].iter().find_map(|k| v.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from))
    });
    let (kind, message) = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            (ProviderErrorKind::Auth, "Vast.ai rejected the API key - check it in Settings → Vast.ai.".to_string())
        }
        StatusCode::TOO_MANY_REQUESTS => (ProviderErrorKind::RateLimit, "Vast.ai is rate limiting requests - try again in a moment.".to_string()),
        StatusCode::NOT_FOUND => (ProviderErrorKind::InvalidRequest, "Vast.ai has no such instance - check the instance ID in Settings → Vast.ai.".to_string()),
        _ => (
            ProviderErrorKind::Unknown,
            match detail {
                Some(d) => format!("Vast.ai request failed ({}): {d}", describe_status(status)),
                None => format!("Vast.ai request failed with status {}.", describe_status(status)),
            },
        ),
    };
    ProviderError::with_status(message, kind, status.as_u16()).user_facing()
}

#[async_trait]
impl InstanceControl for VastApi {
    async fn status(&self, instance_id: i64) -> Result<InstanceStatus, ProviderError> {
        let request = self.client.get(format!("{}/instances/{instance_id}", self.base_url)).bearer_auth(self.key()?);
        let response = Self::check(send(request, NAME).await?).await?;
        let text = response.text().await.map_err(|e| crate::providers::http::transport_error(NAME, &e))?;
        let body: Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::unknown(format!("Failed to parse the Vast.ai instance: {e}")))?;
        // `{"instances": {...}}`; older responses wrap it in a one-element list.
        let instance = match body.get("instances").unwrap_or(&body) {
            Value::Array(list) => list.first().cloned().unwrap_or(Value::Null),
            other => other.clone(),
        };
        if !instance.is_object() {
            return Err(ProviderError::invalid_request("Vast.ai returned no instance - check the instance ID.").user_facing());
        }
        Ok(InstanceStatus::from_json(&instance))
    }

    async fn set_running(&self, instance_id: i64, running: bool) -> Result<(), ProviderError> {
        let state = if running { "running" } else { "stopped" };
        let request = self
            .client
            .put(format!("{}/instances/{instance_id}", self.base_url))
            .bearer_auth(self.key()?)
            .header("content-type", "application/json")
            .body(serde_json::json!({ "state": state }).to_string());
        let response = Self::check(send(request, NAME).await?).await?;
        // A 200 can still carry `{"success": false, "msg": ...}`.
        let body: Value = serde_json::from_str(&response.text().await.unwrap_or_default()).unwrap_or(Value::Null);
        if body.get("success").and_then(Value::as_bool) == Some(false) {
            let msg = body.get("msg").or_else(|| body.get("error")).and_then(Value::as_str).unwrap_or("no reason given");
            return Err(ProviderError::unknown(format!("Vast.ai wouldn't set the instance to {state}: {msg}")).user_facing());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::keys::StaticApiKey;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn api(server: &MockServer) -> VastApi {
        VastApi::new(reqwest::Client::new(), Arc::new(StaticApiKey::some("k"))).with_base_url(server.uri())
    }

    #[test]
    fn endpoint_needs_both_the_ip_and_a_published_port() {
        let status = InstanceStatus::from_json(&json!({
            "actual_status": "running", "intended_status": "running", "public_ipaddr": "1.2.3.4\n",
            "ports": {"11434/tcp": [{"HostIp": "0.0.0.0", "HostPort": "40012"}], "22/tcp": []}
        }));
        assert!(status.is_running());
        assert_eq!(status.endpoint(11434).as_deref(), Some("1.2.3.4:40012"));
        assert_eq!(status.endpoint(22), None);
        assert_eq!(status.endpoint(8080), None);
        assert_eq!(InstanceStatus::from_json(&json!({"ports": null})).endpoint(11434), None);
    }

    #[test]
    fn a_running_container_that_was_told_to_stop_is_not_running() {
        let status = InstanceStatus::from_json(&json!({"actual_status": "running", "intended_status": "stopped"}));
        assert!(!status.is_running());
    }

    #[tokio::test]
    async fn status_unwraps_the_instance_and_sends_the_bearer_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/instances/7"))
            .and(header("authorization", "Bearer k"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"instances": {"actual_status": "loading", "status_msg": "pulling"}})))
            .mount(&server)
            .await;
        let status = api(&server).status(7).await.unwrap();
        assert_eq!(status.actual.as_deref(), Some("loading"));
        assert_eq!(status.message.as_deref(), Some("pulling"));
        assert!(!status.is_running());
    }

    #[tokio::test]
    async fn set_running_puts_the_state() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/instances/7"))
            .and(body_json(json!({"state": "stopped"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .expect(1)
            .mount(&server)
            .await;
        api(&server).set_running(7, false).await.unwrap();
    }

    #[tokio::test]
    async fn refusals_surface_vasts_own_message() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": false, "msg": "GPU is no longer available"})))
            .mount(&server)
            .await;
        let err = api(&server).set_running(7, true).await.unwrap_err();
        assert!(err.message.contains("GPU is no longer available"), "{}", err.message);
    }

    #[tokio::test]
    async fn http_failures_map_to_kinds() {
        let server = MockServer::start().await;
        Mock::given(path("/instances/1")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        Mock::given(path("/instances/2")).respond_with(ResponseTemplate::new(429)).mount(&server).await;
        Mock::given(path("/instances/3")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        let api = api(&server);
        assert_eq!(api.status(1).await.unwrap_err().kind, ProviderErrorKind::Auth);
        assert_eq!(api.status(2).await.unwrap_err().kind, ProviderErrorKind::RateLimit);
        assert_eq!(api.status(3).await.unwrap_err().kind, ProviderErrorKind::InvalidRequest);
    }

    #[tokio::test]
    async fn a_missing_key_is_reported_before_any_request() {
        let api = VastApi::new(reqwest::Client::new(), Arc::new(StaticApiKey::none()));
        assert_eq!(api.status(1).await.unwrap_err().kind, ProviderErrorKind::Auth);
    }
}
