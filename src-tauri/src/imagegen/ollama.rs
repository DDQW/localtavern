//! Making room on a shared graphics card: a local Ollama model and an image server rarely both fit
//! in VRAM, and when they don't, both spill into system RAM and crawl. So before a picture is
//! rendered, whatever Ollama has loaded is unloaded; Ollama loads it again on the next message.

use std::time::Duration;

use serde_json::{json, Value};

const POLL_INTERVAL: Duration = Duration::from_millis(300);
const MAX_WAIT: Duration = Duration::from_secs(15);

async fn loaded_models(client: &reqwest::Client, base_url: &str) -> Option<Vec<String>> {
    let response = client.get(format!("{base_url}/api/ps")).timeout(Duration::from_secs(3)).send().await.ok()?;
    let text = response.error_for_status().ok()?.text().await.ok()?;
    let body: Value = serde_json::from_str(&text).ok()?;
    Some(
        body.get("models")?
            .as_array()?
            .iter()
            .filter_map(|m| m.get("model").or_else(|| m.get("name")).and_then(Value::as_str).map(str::to_string))
            .collect(),
    )
}

/// Unloads every model Ollama at `base_url` has in memory and waits (briefly) until it reports
/// none. Returns the models that were unloaded; an Ollama that isn't running counts as nothing to
/// unload, never as an error - a picture shouldn't fail because of it.
pub async fn unload_all(client: &reqwest::Client, base_url: &str) -> Vec<String> {
    let base_url = base_url.trim().trim_end_matches('/');
    let Some(models) = loaded_models(client, base_url).await.filter(|m| !m.is_empty()) else {
        return Vec::new();
    };
    for model in &models {
        let _ = client
            .post(format!("{base_url}/api/generate"))
            .timeout(Duration::from_secs(30))
            .header("content-type", "application/json")
            .body(json!({ "model": model, "keep_alive": 0 }).to_string())
            .send()
            .await;
    }
    let started = tokio::time::Instant::now();
    while started.elapsed() < MAX_WAIT {
        match loaded_models(client, base_url).await {
            Some(still) if !still.is_empty() => tokio::time::sleep(POLL_INTERVAL).await,
            _ => break,
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn unloads_each_loaded_model_and_waits_until_none_are_left() {
        let server = MockServer::start().await;
        // Loaded at first, gone once unloaded.
        Mock::given(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
                {"name": "big:latest", "model": "big:latest", "size_vram": 1}
            ]})))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": []})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .and(body_partial_json(json!({"model": "big:latest", "keep_alive": 0})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"done": true})))
            .expect(1)
            .mount(&server)
            .await;

        let unloaded = unload_all(&reqwest::Client::new(), &format!("{}/", server.uri())).await;
        assert_eq!(unloaded, ["big:latest"]);
    }

    #[tokio::test]
    async fn nothing_loaded_or_no_ollama_is_not_an_error() {
        let server = MockServer::start().await;
        Mock::given(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": []})))
            .mount(&server)
            .await;
        Mock::given(path("/api/generate")).respond_with(ResponseTemplate::new(500)).expect(0).mount(&server).await;
        assert!(unload_all(&reqwest::Client::new(), &server.uri()).await.is_empty());
        assert!(unload_all(&reqwest::Client::new(), "http://127.0.0.1:9").await.is_empty());
    }
}
