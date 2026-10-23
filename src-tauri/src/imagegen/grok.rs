//! Grok Imagine through xAI's images API, signed in with the SuperGrok subscription (the same OAuth
//! session the Grok chat provider uses). It renders in the cloud from a sentence prompt and takes
//! only a model, an aspect ratio and a resolution - steps, CFG, seed, sampler and the negative
//! prompt have nowhere to go.

use base64::Engine;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{sniff_mime, unreachable, BackendOptions, GeneratedImage, ImageRequest};
use crate::error::{Error, Result};
use crate::providers::auth::AccessTokenSource;
use crate::providers::xai::errors::to_provider_error;

const SERVER: &str = "xAI";

/// Used when Settings leave the model empty, and as the list when xAI doesn't list its image models.
const KNOWN_MODELS: [&str; 3] = ["grok-imagine-image-2.0", "grok-imagine-image-quality", "grok-imagine-image"];

/// The ratios the API accepts ("auto" aside); the picture's size is snapped to the nearest one.
const ASPECT_RATIOS: [(u32, u32); 13] =
    [(1, 1), (16, 9), (9, 16), (4, 3), (3, 4), (3, 2), (2, 3), (2, 1), (1, 2), (20, 9), (9, 20), (21, 9), (5, 2)];

#[derive(Deserialize)]
struct ModelList {
    #[serde(default, alias = "data")]
    models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(Deserialize)]
struct ImagesResponse {
    #[serde(default)]
    data: Vec<ImageData>,
}

#[derive(Deserialize)]
struct ImageData {
    b64_json: Option<String>,
    /// Set to false when moderation held the picture back.
    respect_moderation: Option<bool>,
}

pub fn aspect_ratio(width: u32, height: u32) -> String {
    let target = (width.max(1) as f64 / height.max(1) as f64).ln();
    let (w, h) = ASPECT_RATIOS
        .iter()
        .min_by(|a, b| {
            let distance = |(w, h): &&(u32, u32)| ((*w as f64 / *h as f64).ln() - target).abs();
            distance(a).total_cmp(&distance(b))
        })
        .expect("the list is not empty");
    format!("{w}:{h}")
}

/// xAI renders at about one or two megapixels; anything bigger than a regular SDXL size asks for 2k.
pub fn resolution(width: u32, height: u32) -> &'static str {
    if width.max(height) > 1536 { "2k" } else { "1k" }
}

pub fn generation_body(request: &ImageRequest) -> Value {
    let model = if request.model.is_empty() { KNOWN_MODELS[0] } else { &request.model };
    json!({
        "model": model,
        "prompt": request.prompt,
        "n": 1,
        "aspect_ratio": aspect_ratio(request.width, request.height),
        "resolution": resolution(request.width, request.height),
        "response_format": "b64_json",
    })
}

pub async fn options(client: &reqwest::Client, url: &str, auth: &dyn AccessTokenSource) -> Result<BackendOptions> {
    let token = auth.access_token().await?;
    let response = client
        .get(format!("{url}/image-generation-models"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| unreachable(SERVER, url, &e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| unreachable(SERVER, url, &e))?;
    let known = || KNOWN_MODELS.map(String::from).to_vec();
    let models = if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
        known()
    } else if !status.is_success() {
        return Err(to_provider_error(status, &body).into());
    } else {
        let listed: Vec<String> = serde_json::from_str::<ModelList>(&body)
            .map(|l| l.models.into_iter().map(|m| m.id).collect())
            .unwrap_or_default();
        if listed.is_empty() { known() } else { listed }
    };
    Ok(BackendOptions { models, ..Default::default() })
}

pub async fn generate(
    client: &reqwest::Client,
    url: &str,
    auth: &dyn AccessTokenSource,
    request: &ImageRequest,
) -> Result<GeneratedImage> {
    let token = auth.access_token().await?;
    let response = client
        .post(format!("{url}/images/generations"))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(generation_body(request).to_string())
        .send()
        .await
        .map_err(|e| unreachable(SERVER, url, &e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| unreachable(SERVER, url, &e))?;
    if !status.is_success() {
        return Err(to_provider_error(status, &body).into());
    }
    let parsed: ImagesResponse = serde_json::from_str(&body)
        .map_err(|e| Error::Other(format!("Unexpected answer from {SERVER}: {e}")))?;
    let encoded = parsed
        .data
        .into_iter()
        .next()
        .filter(|d| d.respect_moderation != Some(false))
        .and_then(|d| d.b64_json)
        .filter(|b| !b.trim().is_empty())
        .ok_or_else(|| Error::Other("Grok sent no picture back - xAI's moderation may have blocked this prompt.".into()))?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| Error::Other(format!("{SERVER} returned a picture that isn't valid base64: {e}")))?;
    let mime_type = sniff_mime(&data).to_string();
    Ok(GeneratedImage { data, mime_type })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::auth::StaticAccessToken;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];

    fn request() -> ImageRequest {
        ImageRequest {
            prompt: "a woman with silver hair in a cafe".into(),
            negative_prompt: "blurry".into(),
            width: 832,
            height: 1216,
            steps: 25,
            cfg_scale: 7.0,
            seed: 42,
            model: String::new(),
            sampler: String::new(),
            scheduler: String::new(),
        }
    }

    #[test]
    fn sizes_snap_to_the_nearest_supported_ratio() {
        assert_eq!(aspect_ratio(832, 1216), "2:3");
        assert_eq!(aspect_ratio(1216, 832), "3:2");
        assert_eq!(aspect_ratio(1024, 1024), "1:1");
        assert_eq!(aspect_ratio(1344, 768), "16:9");
        assert_eq!(aspect_ratio(0, 0), "1:1");
        assert_eq!((resolution(832, 1216), resolution(2048, 1152)), ("1k", "2k"));
    }

    #[test]
    fn body_names_a_model_and_leaves_out_what_grok_cannot_use() {
        let body = generation_body(&request());
        assert_eq!(body["model"], "grok-imagine-image-2.0");
        assert_eq!((body["aspect_ratio"].as_str(), body["response_format"].as_str()), (Some("2:3"), Some("b64_json")));
        for unused in ["negative_prompt", "seed", "steps"] {
            assert!(body.get(unused).is_none(), "{unused}");
        }
        assert_eq!(generation_body(&ImageRequest { model: "grok-imagine-image".into(), ..request() })["model"], "grok-imagine-image");
    }

    #[tokio::test]
    async fn generates_with_the_subscription_token() {
        let server = MockServer::start().await;
        let b64 = base64::engine::general_purpose::STANDARD.encode(JPEG);
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .and(header("authorization", "Bearer AT"))
            .and(body_partial_json(json!({ "prompt": "a woman with silver hair in a cafe", "n": 1 })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "b64_json": b64 }] })))
            .expect(1)
            .mount(&server)
            .await;

        let image = generate(&reqwest::Client::new(), &server.uri(), &StaticAccessToken::new("AT"), &request()).await.unwrap();
        assert_eq!((image.data.as_slice(), image.mime_type.as_str()), (JPEG, "image/jpeg"));
    }

    #[tokio::test]
    async fn a_held_back_picture_is_explained() {
        for answer in [json!({ "data": [] }), json!({ "data": [{ "b64_json": "AAAA", "respect_moderation": false }] })] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(answer)).mount(&server).await;
            let err = generate(&reqwest::Client::new(), &server.uri(), &StaticAccessToken::new("AT"), &request()).await.unwrap_err();
            assert!(err.to_string().contains("moderation"), "{err}");
        }
    }

    #[tokio::test]
    async fn a_plan_without_api_access_gets_the_403_explanation() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(403).set_body_raw("{}", "application/json")).mount(&server).await;
        let err = generate(&reqwest::Client::new(), &server.uri(), &StaticAccessToken::new("AT"), &request()).await.unwrap_err();
        assert!(err.to_string().contains("HTTP 403"), "{err}");
    }

    #[tokio::test]
    async fn options_list_xais_image_models_or_fall_back_to_the_known_ones() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/image-generation-models"))
            .and(header("authorization", "Bearer AT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [{ "id": "grok-imagine-image" }] })))
            .mount(&server)
            .await;
        let listed = options(&reqwest::Client::new(), &server.uri(), &StaticAccessToken::new("AT")).await.unwrap();
        assert_eq!((listed.models, listed.samplers.len()), (vec!["grok-imagine-image".to_string()], 0));

        let missing = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(404)).mount(&missing).await;
        let known = options(&reqwest::Client::new(), &missing.uri(), &StaticAccessToken::new("AT")).await.unwrap();
        assert_eq!(known.models, KNOWN_MODELS);
    }
}
