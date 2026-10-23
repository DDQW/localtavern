//! The `/sdapi/v1` API shared by AUTOMATIC1111, Forge, reForge and SD.Next. It only exists when
//! the server was started with `--api`, so a 404 there gets its own explanation.

use base64::Engine;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{excerpt, sniff_mime, unreachable, BackendOptions, GeneratedImage, ImageRequest};
use crate::error::{Error, Result};

const SERVER: &str = "Stable Diffusion WebUI";

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct Model {
    title: String,
}

#[derive(Deserialize)]
struct Txt2ImgResponse {
    #[serde(default)]
    images: Vec<String>,
}

fn status_error(url: &str, status: StatusCode, body: &str) -> Error {
    if status == StatusCode::NOT_FOUND {
        return Error::Other(format!(
            "{SERVER} at {url} has no API. Start it with the --api flag (add it to COMMANDLINE_ARGS in webui-user.bat)."
        ));
    }
    // FastAPI errors look like {"detail": "..."} or {"error": "...", "errors": "..."}.
    let detail = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        ["detail", "errors", "error"]
            .iter()
            .find_map(|k| v.get(k).and_then(Value::as_str).map(str::to_string))
    });
    Error::Other(format!(
        "{SERVER} returned {status}: {}",
        detail.unwrap_or_else(|| excerpt(body))
    ))
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    url: &str,
    path: &str,
) -> Result<T> {
    let response = client
        .get(format!("{url}{path}"))
        .send()
        .await
        .map_err(|e| unreachable(SERVER, url, &e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| unreachable(SERVER, url, &e))?;
    if !status.is_success() {
        return Err(status_error(url, status, &body));
    }
    serde_json::from_str(&body)
        .map_err(|e| Error::Other(format!("Unexpected answer from {SERVER} ({path}): {e}")))
}

pub async fn options(client: &reqwest::Client, url: &str) -> Result<BackendOptions> {
    let models: Vec<Model> = get_json(client, url, "/sdapi/v1/sd-models").await?;
    let samplers: Vec<Named> = get_json(client, url, "/sdapi/v1/samplers").await?;
    // Schedulers are a separate list only on newer builds (A1111 1.9+, Forge); older ones fold
    // them into the sampler name.
    let schedulers: Vec<Named> = get_json(client, url, "/sdapi/v1/schedulers")
        .await
        .unwrap_or_default();
    Ok(BackendOptions {
        models: models.into_iter().map(|m| m.title).collect(),
        samplers: samplers.into_iter().map(|s| s.name).collect(),
        schedulers: schedulers.into_iter().map(|s| s.name).collect(),
    })
}

pub fn txt2img_body(request: &ImageRequest) -> Value {
    let mut body = json!({
        "prompt": request.prompt,
        "negative_prompt": request.negative_prompt,
        "width": request.width,
        "height": request.height,
        "steps": request.steps,
        "cfg_scale": request.cfg_scale,
        "seed": request.seed,
        "batch_size": 1,
        "n_iter": 1,
        "send_images": true,
        "save_images": false,
    });
    if !request.sampler.is_empty() {
        body["sampler_name"] = json!(request.sampler);
    }
    if !request.scheduler.is_empty() {
        body["scheduler"] = json!(request.scheduler);
    }
    if !request.model.is_empty() {
        // Switches checkpoint for this picture only (loading one takes a while the first time).
        body["override_settings"] = json!({ "sd_model_checkpoint": request.model });
        body["override_settings_restore_afterwards"] = json!(false);
    }
    body
}

pub async fn generate(
    client: &reqwest::Client,
    url: &str,
    request: &ImageRequest,
) -> Result<GeneratedImage> {
    let response = client
        .post(format!("{url}/sdapi/v1/txt2img"))
        .header("content-type", "application/json")
        .body(txt2img_body(request).to_string())
        .send()
        .await
        .map_err(|e| unreachable(SERVER, url, &e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| unreachable(SERVER, url, &e))?;
    if !status.is_success() {
        return Err(status_error(url, status, &body));
    }
    let parsed: Txt2ImgResponse = serde_json::from_str(&body)
        .map_err(|e| Error::Other(format!("Unexpected answer from {SERVER}: {e}")))?;
    let first = parsed
        .images
        .into_iter()
        .next()
        .ok_or_else(|| Error::Other(format!("{SERVER} returned no picture.")))?;
    // Some forks prefix a data URL.
    let encoded = first
        .split_once("base64,")
        .map(|(_, b)| b)
        .unwrap_or(&first);
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| {
            Error::Other(format!(
                "{SERVER} returned a picture that isn't valid base64: {e}"
            ))
        })?;
    let mime_type = sniff_mime(&data).to_string();
    Ok(GeneratedImage { data, mime_type })
}

/// Best-effort: asks the server to stop the picture in progress.
pub async fn interrupt(client: &reqwest::Client, url: &str) {
    let _ = client
        .post(format!("{url}/sdapi/v1/interrupt"))
        .send()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2, 3];

    fn request() -> ImageRequest {
        ImageRequest {
            prompt: "a cat".into(),
            negative_prompt: "blurry".into(),
            width: 512,
            height: 768,
            steps: 20,
            cfg_scale: 6.5,
            seed: -1,
            model: String::new(),
            sampler: "Euler a".into(),
            scheduler: String::new(),
        }
    }

    #[test]
    fn body_leaves_out_unset_choices() {
        let body = txt2img_body(&request());
        assert_eq!(body["sampler_name"], "Euler a");
        assert!(body.get("scheduler").is_none());
        assert!(body.get("override_settings").is_none());

        let body = txt2img_body(&ImageRequest {
            model: "sdxl.safetensors [abc]".into(),
            scheduler: "Karras".into(),
            ..request()
        });
        assert_eq!(
            body["override_settings"]["sd_model_checkpoint"],
            "sdxl.safetensors [abc]"
        );
        assert_eq!(body["scheduler"], "Karras");
    }

    #[tokio::test]
    async fn generates_and_decodes_the_first_picture() {
        let server = MockServer::start().await;
        let b64 = base64::engine::general_purpose::STANDARD.encode(PNG);
        Mock::given(method("POST"))
            .and(path("/sdapi/v1/txt2img"))
            .and(body_partial_json(
                json!({"prompt": "a cat", "width": 512, "height": 768, "steps": 20}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"images": [b64], "info": "{}"})),
            )
            .mount(&server)
            .await;

        let image = generate(&client(), &server.uri(), &request())
            .await
            .unwrap();
        assert_eq!(image.data, PNG);
        assert_eq!(image.mime_type, "image/png");
    }

    #[tokio::test]
    async fn accepts_a_data_url_prefix() {
        let server = MockServer::start().await;
        let b64 = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(PNG)
        );
        Mock::given(path("/sdapi/v1/txt2img"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"images": [b64]})))
            .mount(&server)
            .await;
        assert_eq!(
            generate(&client(), &server.uri(), &request())
                .await
                .unwrap()
                .data,
            PNG
        );
    }

    #[tokio::test]
    async fn a_missing_api_says_to_use_the_api_flag() {
        let server = MockServer::start().await; // nothing mounted: every path is a 404
        let error = generate(&client(), &server.uri(), &request())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("--api"), "{error}");
    }

    #[tokio::test]
    async fn server_errors_show_their_detail() {
        let server = MockServer::start().await;
        Mock::given(path("/sdapi/v1/txt2img"))
            .respond_with(
                ResponseTemplate::new(422)
                    .set_body_json(json!({"detail": "Sampler not found: Bogus"})),
            )
            .mount(&server)
            .await;
        let error = generate(&client(), &server.uri(), &request())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("Sampler not found: Bogus"), "{error}");
    }

    #[tokio::test]
    async fn a_stopped_server_is_reported_as_unreachable() {
        // Port 9 (discard) is essentially never listening on a desktop.
        let error = generate(&client(), "http://127.0.0.1:9", &request())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("is it running?"), "{error}");
    }

    #[tokio::test]
    async fn lists_models_samplers_and_optional_schedulers() {
        let server = MockServer::start().await;
        Mock::given(path("/sdapi/v1/sd-models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"title": "a.safetensors [1]", "model_name": "a"}])),
            )
            .mount(&server)
            .await;
        Mock::given(path("/sdapi/v1/samplers"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!([{"name": "Euler a", "aliases": []}, {"name": "DPM++ 2M"}]),
                ),
            )
            .mount(&server)
            .await;

        let listed = options(&client(), &server.uri()).await.unwrap();
        assert_eq!(listed.models, ["a.safetensors [1]"]);
        assert_eq!(listed.samplers, ["Euler a", "DPM++ 2M"]);
        assert!(
            listed.schedulers.is_empty(),
            "an older build without /schedulers still works"
        );
    }

    fn client() -> reqwest::Client {
        super::super::client()
    }
}
