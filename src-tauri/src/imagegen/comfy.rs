//! ComfyUI: queue a workflow (`POST /prompt`), wait for it in `/history`, fetch the picture from
//! `/view`. The built-in still workflow is plain checkpoint text-to-image; the built-in video
//! workflow is Wan text-to-video. A custom one (exported with "Export (API)") can use the same
//! `%placeholder%` names SillyTavern uses.

use std::time::Duration;

use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::{excerpt, random_seed, sniff_mime, unreachable, BackendOptions, GeneratedImage, ImageRequest};
use crate::error::{Error, Result};

const SERVER: &str = "ComfyUI";
const POLL_INTERVAL: Duration = Duration::from_millis(750);
/// A queued picture may wait behind others; give up after this long.
const MAX_WAIT: Duration = Duration::from_secs(20 * 60);

pub const DEFAULT_WORKFLOW: &str = r#"{
  "3": { "class_type": "KSampler", "inputs": {
    "seed": "%seed%", "steps": "%steps%", "cfg": "%scale%", "sampler_name": "%sampler%", "scheduler": "%scheduler%",
    "denoise": 1, "model": ["4", 0], "positive": ["6", 0], "negative": ["7", 0], "latent_image": ["5", 0] } },
  "4": { "class_type": "CheckpointLoaderSimple", "inputs": { "ckpt_name": "%model%" } },
  "5": { "class_type": "EmptyLatentImage", "inputs": { "width": "%width%", "height": "%height%", "batch_size": 1 } },
  "6": { "class_type": "CLIPTextEncode", "inputs": { "text": "%prompt%", "clip": ["4", 1] } },
  "7": { "class_type": "CLIPTextEncode", "inputs": { "text": "%negative_prompt%", "clip": ["4", 1] } },
  "8": { "class_type": "VAEDecode", "inputs": { "samples": ["3", 0], "vae": ["4", 2] } },
  "9": { "class_type": "SaveImage", "inputs": { "filename_prefix": "LocalTavern", "images": ["8", 0] } }
}"#;

/// Built-in story clip: ComfyUI's native Wan 2.1 text-to-video graph, saved as WebM so the chat
/// can play it. `%unet%` `%clip%` `%vae%` are filled from the models ComfyUI actually has.
/// 832×480 and 33 frames match the official 1.3B example, so a shared graphics card has a chance.
pub const DEFAULT_VIDEO_WORKFLOW: &str = r#"{
  "37": { "class_type": "UNETLoader", "inputs": { "unet_name": "%unet%", "weight_dtype": "default" } },
  "38": { "class_type": "CLIPLoader", "inputs": { "clip_name": "%clip%", "type": "wan" } },
  "39": { "class_type": "VAELoader", "inputs": { "vae_name": "%vae%" } },
  "48": { "class_type": "ModelSamplingSD3", "inputs": { "shift": 8, "model": ["37", 0] } },
  "6": { "class_type": "CLIPTextEncode", "inputs": { "text": "%prompt%, dynamic motion, the subject is moving", "clip": ["38", 0] } },
  "7": { "class_type": "CLIPTextEncode", "inputs": { "text": "%negative_prompt%, static, blurry, watermark, text, subtitle, motionless", "clip": ["38", 0] } },
  "40": { "class_type": "EmptyHunyuanLatentVideo", "inputs": { "width": 832, "height": 480, "length": 33, "batch_size": 1 } },
  "3": { "class_type": "KSampler", "inputs": {
    "seed": "%seed%", "steps": 20, "cfg": 6, "sampler_name": "uni_pc", "scheduler": "simple",
    "denoise": 1, "model": ["48", 0], "positive": ["6", 0], "negative": ["7", 0], "latent_image": ["40", 0] } },
  "8": { "class_type": "VAEDecode", "inputs": { "samples": ["3", 0], "vae": ["39", 0] } },
  "47": { "class_type": "SaveWEBM", "inputs": { "images": ["8", 0], "filename_prefix": "LocalTavern", "codec": "vp9", "fps": 16, "crf": 32 } }
}"#;

/// Fills a workflow's placeholders. A string that is exactly a numeric placeholder (`"%seed%"`)
/// becomes a JSON number; placeholders inside longer strings are replaced as text.
pub fn fill_workflow(workflow: &str, request: &ImageRequest) -> Result<Value> {
    fill_workflow_with(workflow, request, &[])
}

/// `extra` are more `%name%` → text replacements, such as the video model filenames.
pub fn fill_workflow_with(workflow: &str, request: &ImageRequest, extra: &[(&str, &str)]) -> Result<Value> {
    let mut value: Value = serde_json::from_str(workflow.trim())
        .map_err(|e| Error::Other(format!("The ComfyUI workflow isn't valid JSON: {e}")))?;
    let Some(nodes) = value.as_object() else {
        return Err(Error::Other("The ComfyUI workflow must be a JSON object of nodes.".into()));
    };
    if nodes.contains_key("nodes") && nodes.contains_key("links") {
        return Err(Error::Other(
            "That's a ComfyUI UI workflow. Save it with Workflow → Export (API) and paste that instead.".into(),
        ));
    }

    let seed = if request.seed < 0 { random_seed() } else { request.seed };
    let numbers: [(&str, Value); 5] = [
        ("%seed%", json!(seed)),
        ("%steps%", json!(request.steps)),
        ("%scale%", json!(request.cfg_scale)),
        ("%width%", json!(request.width)),
        ("%height%", json!(request.height)),
    ];
    // `%negative_prompt%` stays ahead of `%prompt%` so the shorter name can't eat its tail.
    let mut texts: Vec<(&str, &str)> = Vec::with_capacity(5 + extra.len());
    texts.extend_from_slice(extra);
    texts.extend_from_slice(&[
        ("%negative_prompt%", request.negative_prompt.as_str()),
        ("%prompt%", request.prompt.as_str()),
        ("%model%", request.model.as_str()),
        ("%sampler%", request.sampler.as_str()),
        ("%scheduler%", request.scheduler.as_str()),
    ]);

    fn walk(value: &mut Value, numbers: &[(&str, Value)], texts: &[(&str, &str)]) {
        match value {
            Value::String(s) => {
                if let Some((_, number)) = numbers.iter().find(|(name, _)| s == name) {
                    *value = number.clone();
                    return;
                }
                if s.contains('%') {
                    let mut filled = s.clone();
                    // `%negative_prompt%` goes first so `%prompt%` can't eat its tail.
                    for (name, text) in texts {
                        filled = filled.replace(name, text);
                    }
                    for (name, number) in numbers {
                        filled = filled.replace(name, &number.to_string());
                    }
                    *s = filled;
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|v| walk(v, numbers, texts)),
            Value::Object(map) => map.values_mut().for_each(|v| walk(v, numbers, texts)),
            _ => {}
        }
    }
    walk(&mut value, &numbers, &texts);
    Ok(value)
}

/// The choices of a combo input in `/object_info`: older ComfyUI sends `[[...choices], {...}]`,
/// newer sends `["COMBO", {"options": [...]}]`.
fn combo_choices(info: &Value, node: &str, input: &str) -> Vec<String> {
    let Some(spec) = info.pointer(&format!("/{node}/input/required/{input}")) else {
        return Vec::new();
    };
    let list = match spec.get(0) {
        Some(Value::Array(items)) => Some(items),
        Some(Value::String(kind)) if kind == "COMBO" => spec.pointer("/1/options").and_then(Value::as_array),
        _ => None,
    };
    list.map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

async fn get_json(client: &reqwest::Client, url: &str, path: &str) -> Result<Value> {
    let response = client.get(format!("{url}{path}")).send().await.map_err(|e| unreachable(SERVER, url, &e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| unreachable(SERVER, url, &e))?;
    if !status.is_success() {
        return Err(Error::Other(format!("{SERVER} returned {status} for {path}: {}", excerpt(&body))));
    }
    serde_json::from_str(&body).map_err(|e| Error::Other(format!("Unexpected answer from {SERVER} ({path}): {e}")))
}

pub async fn options(client: &reqwest::Client, url: &str) -> Result<BackendOptions> {
    let checkpoints = get_json(client, url, "/object_info/CheckpointLoaderSimple").await?;
    let sampler = get_json(client, url, "/object_info/KSampler").await?;
    Ok(BackendOptions {
        models: combo_choices(&checkpoints, "CheckpointLoaderSimple", "ckpt_name"),
        samplers: combo_choices(&sampler, "KSampler", "sampler_name"),
        schedulers: combo_choices(&sampler, "KSampler", "scheduler"),
    })
}

/// ComfyUI's reason for refusing a workflow: `{"error": {"message", "details"}, "node_errors": {...}}`.
fn queue_error(status: reqwest::StatusCode, body: &str) -> Error {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Error::Other(format!("{SERVER} refused the workflow ({status}): {}", excerpt(body)));
    };
    let mut parts = Vec::new();
    if let Some(message) = v.pointer("/error/message").and_then(Value::as_str) {
        parts.push(message.to_string());
    }
    if let Some(nodes) = v.get("node_errors").and_then(Value::as_object) {
        for (id, node) in nodes {
            for error in node.get("errors").and_then(Value::as_array).into_iter().flatten() {
                let details = error.get("details").and_then(Value::as_str).unwrap_or_default();
                let message = error.get("message").and_then(Value::as_str).unwrap_or_default();
                let class = node.get("class_type").and_then(Value::as_str).unwrap_or("node");
                parts.push(format!("{class} #{id}: {message} {details}").trim().to_string());
            }
        }
    }
    if parts.is_empty() {
        parts.push(excerpt(body));
    }
    Error::Other(format!("{SERVER} refused the workflow: {}", parts.join("; ")))
}

/// The first media file in a finished history entry. Prefers video/`gifs` outputs (VHS etc.), then
/// still images; saved outputs over previews.
fn first_media(entry: &Value) -> Option<Map<String, Value>> {
    let outputs = entry.get("outputs")?.as_object()?;
    let pick = |key: &str| -> Vec<&Map<String, Value>> {
        outputs
            .values()
            .filter_map(|o| o.get(key).and_then(Value::as_array))
            .flatten()
            .filter_map(Value::as_object)
            .collect()
    };
    for key in ["gifs", "videos", "images"] {
        let files = pick(key);
        if let Some(file) = files.iter().find(|i| i.get("type").and_then(Value::as_str) == Some("output")).or_else(|| files.first())
        {
            return Some((*file).clone());
        }
    }
    None
}

/// Why a run failed, from its history status messages.
fn execution_error(entry: &Value) -> Option<String> {
    let status = entry.get("status")?;
    if status.get("status_str").and_then(Value::as_str) != Some("error") {
        return None;
    }
    let message = status
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|m| m.get(0).and_then(Value::as_str) == Some("execution_error"))
        .find_map(|m| m.pointer("/1/exception_message").and_then(Value::as_str))
        .unwrap_or("the workflow failed")
        .trim()
        .to_string();
    Some(message)
}

/// The diffusion model, text encoder and VAE the built-in video workflow should load.
struct VideoAssets {
    unet: String,
    clip: String,
    vae: String,
}

/// Highest score wins; a tie goes to the shorter name, then lexicographic, so the pick is stable.
fn rank_pick<'a>(names: &'a [String], rank: impl Fn(&str) -> i32) -> Option<&'a str> {
    names
        .iter()
        .filter_map(|name| {
            let score = rank(name);
            (score > 0).then_some((score, name.as_str()))
        })
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.len().cmp(&a.1.len())).then(b.1.cmp(a.1)))
        .map(|(_, name)| name)
}

/// Prefer a small Wan text-to-video model. Image-to-video and still checkpoints score zero.
fn unet_rank(name: &str) -> i32 {
    let n = name.to_ascii_lowercase();
    if n.contains("i2v") && !n.contains("t2v") {
        return 0;
    }
    let mut score = if n.contains("t2v") || n.contains("text2video") || n.contains("text_to_video") {
        100
    } else if n.contains("wan") {
        40
    } else {
        return 0;
    };
    if n.contains("1.3b") || n.contains("1_3b") || n.contains("1-3b") {
        score += 20;
    }
    if n.contains("fp8") {
        score += 5;
    }
    score
}

fn clip_rank(name: &str) -> i32 {
    let n = name.to_ascii_lowercase();
    let mut score = if n.contains("umt5") {
        100
    } else if n.contains("wan") {
        40
    } else if n.contains("t5") {
        20
    } else {
        return 0;
    };
    if n.contains("fp8") {
        score += 5;
    }
    score
}

fn vae_rank(name: &str) -> i32 {
    let n = name.to_ascii_lowercase();
    if n == "pixel_space" || n.starts_with("tae") {
        return 0;
    }
    if n.contains("wan") { 100 } else { 0 }
}

/// Combo choices for one node. A missing node (older ComfyUI) is an empty list, not a failure.
async fn node_choices(client: &reqwest::Client, url: &str, node: &str, input: &str) -> Result<Vec<String>> {
    match get_json(client, url, &format!("/object_info/{node}")).await {
        Ok(info) => Ok(combo_choices(&info, node, input)),
        Err(e) if e.to_string().contains("404") => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// `Ok(None)` means ComfyUI has no Wan stack. A story clip then uses the still workflow instead of
/// failing: a photo checkpoint cannot make a video, and the shot should still show up.
async fn video_assets(client: &reqwest::Client, url: &str) -> Result<Option<VideoAssets>> {
    let unets = node_choices(client, url, "UNETLoader", "unet_name").await?;
    let clips = node_choices(client, url, "CLIPLoader", "clip_name").await?;
    let vaes = node_choices(client, url, "VAELoader", "vae_name").await?;
    let unet = rank_pick(&unets, unet_rank);
    let clip = rank_pick(&clips, clip_rank);
    let vae = rank_pick(&vaes, vae_rank);
    match (unet, clip, vae) {
        (Some(unet), Some(clip), Some(vae)) => Ok(Some(VideoAssets { unet: unet.to_string(), clip: clip.to_string(), vae: vae.to_string() })),
        _ => Ok(None),
    }
}

pub async fn generate(
    client: &reqwest::Client,
    url: &str,
    custom_workflow: &str,
    request: &ImageRequest,
    video: bool,
) -> Result<GeneratedImage> {
    // A pasted workflow always wins. Otherwise a clip uses Wan when those models are installed,
    // and the ordinary checkpoint workflow when they are not (a photo model cannot make a video).
    let pasted = !custom_workflow.trim().is_empty();
    let assets = if video && (!pasted || custom_workflow.contains("%unet%")) {
        video_assets(client, url).await?
    } else {
        None
    };
    let template = if pasted {
        custom_workflow
    } else if video && assets.is_some() {
        DEFAULT_VIDEO_WORKFLOW
    } else {
        DEFAULT_WORKFLOW
    };
    let mut request = request.clone();
    if request.model.is_empty() && template.contains("%model%") {
        // Nothing picked yet: use the first checkpoint ComfyUI knows about.
        request.model = options(client, url).await?.models.into_iter().next().ok_or_else(|| {
            Error::Other(format!("{SERVER} has no checkpoints - put one in ComfyUI's models/checkpoints folder."))
        })?;
    }
    let extra: Vec<(&str, &str)> = match &assets {
        Some(assets) => vec![("%unet%", assets.unet.as_str()), ("%clip%", assets.clip.as_str()), ("%vae%", assets.vae.as_str())],
        None => Vec::new(),
    };
    let workflow = fill_workflow_with(template, &request, &extra)?;

    let response = client
        .post(format!("{url}/prompt"))
        .header("content-type", "application/json")
        .body(json!({ "prompt": workflow, "client_id": Uuid::new_v4().to_string() }).to_string())
        .send()
        .await
        .map_err(|e| unreachable(SERVER, url, &e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| unreachable(SERVER, url, &e))?;
    if !status.is_success() {
        return Err(queue_error(status, &body));
    }
    let queued: Value = serde_json::from_str(&body).map_err(|e| Error::Other(format!("Unexpected answer from {SERVER}: {e}")))?;
    let prompt_id = queued
        .get("prompt_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Other(format!("{SERVER} didn't say which job it queued.")))?
        .to_string();

    let started = tokio::time::Instant::now();
    let image = loop {
        let history = get_json(client, url, &format!("/history/{prompt_id}")).await?;
        if let Some(entry) = history.get(&prompt_id) {
            if let Some(error) = execution_error(entry) {
                return Err(Error::Other(format!("{SERVER} failed: {error}")));
            }
            let done = entry.pointer("/status/completed").and_then(Value::as_bool).unwrap_or(true);
            if done {
                break first_media(entry).ok_or_else(|| {
                    Error::Other(format!(
                        "The {SERVER} workflow finished without a picture or video - does it end in Save Image / Video Combine?"
                    ))
                })?;
            }
        }
        if started.elapsed() > MAX_WAIT {
            return Err(Error::Other(format!("Gave up waiting for {SERVER} after {} minutes.", MAX_WAIT.as_secs() / 60)));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    };

    let field = |k: &str| image.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let view = reqwest::Url::parse_with_params(
        &format!("{url}/view"),
        &[("filename", field("filename")), ("subfolder", field("subfolder")), ("type", field("type"))],
    )
    .map_err(|e| Error::Other(format!("Bad ComfyUI address {url}: {e}")))?;
    let response = client
        .get(view)
        .send()
        .await
        .map_err(|e| unreachable(SERVER, url, &e))?;
    if !response.status().is_success() {
        return Err(Error::Other(format!("{SERVER} couldn't return the finished picture ({}).", response.status())));
    }
    let data = response.bytes().await.map_err(|e| unreachable(SERVER, url, &e))?.to_vec();
    let mime_type = sniff_mime(&data).to_string();
    Ok(GeneratedImage { data, mime_type })
}

/// Best-effort: moves ComfyUI's models off the graphics card. They stay cached in RAM, so the next
/// picture is back on the card in a second or two. Needed on a shared card: ComfyUI otherwise keeps
/// ~10 GB of VRAM while idle (its memory manager doesn't release it on AMD).
pub async fn free_vram(client: &reqwest::Client, url: &str) {
    let _ = client
        .post(format!("{url}/free"))
        .header("content-type", "application/json")
        .body(json!({ "unload_models": true }).to_string())
        .send()
        .await;
}

/// Best-effort: stops whatever ComfyUI is running.
pub async fn interrupt(client: &reqwest::Client, url: &str) {
    let _ = client.post(format!("{url}/interrupt")).send().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 9, 9];

    fn request() -> ImageRequest {
        ImageRequest {
            prompt: "a \"quoted\" cat".into(),
            negative_prompt: "blurry".into(),
            width: 832,
            height: 1216,
            steps: 25,
            cfg_scale: 7.5,
            seed: 42,
            model: "sdxl.safetensors".into(),
            sampler: "euler".into(),
            scheduler: "normal".into(),
        }
    }

    #[test]
    fn the_default_workflow_fills_numbers_as_numbers_and_text_as_text() {
        let w = fill_workflow(DEFAULT_WORKFLOW, &request()).unwrap();
        assert_eq!(w["3"]["inputs"]["seed"], 42);
        assert_eq!(w["3"]["inputs"]["steps"], 25);
        assert_eq!(w["3"]["inputs"]["cfg"], 7.5);
        assert_eq!(w["5"]["inputs"]["width"], 832);
        assert_eq!(w["6"]["inputs"]["text"], "a \"quoted\" cat", "quotes survive because we fill parsed JSON");
        assert_eq!(w["7"]["inputs"]["text"], "blurry");
        assert_eq!(w["4"]["inputs"]["ckpt_name"], "sdxl.safetensors");
        assert_eq!(w["3"]["inputs"]["model"], json!(["4", 0]), "links are untouched");
    }

    #[test]
    fn a_random_seed_is_picked_when_none_is_set() {
        let w = fill_workflow(DEFAULT_WORKFLOW, &ImageRequest { seed: -1, ..request() }).unwrap();
        let seed = w["3"]["inputs"]["seed"].as_i64().unwrap();
        assert!((0..1 << 48).contains(&seed));
    }

    #[test]
    fn placeholders_inside_longer_strings_are_replaced_as_text() {
        let w = fill_workflow(r#"{"1": {"inputs": {"text": "%prompt%, seed %seed%, not %negative_prompt%"}}}"#, &request()).unwrap();
        assert_eq!(w["1"]["inputs"]["text"], "a \"quoted\" cat, seed 42, not blurry");
    }

    #[test]
    fn rejects_bad_and_ui_format_workflows() {
        assert!(fill_workflow("{oops", &request()).unwrap_err().to_string().contains("valid JSON"));
        assert!(fill_workflow("[1, 2]", &request()).is_err());
        let ui = r#"{"nodes": [], "links": [], "version": 0.4}"#;
        assert!(fill_workflow(ui, &request()).unwrap_err().to_string().contains("Export (API)"));
    }

    #[test]
    fn reads_combo_choices_in_old_and_new_formats() {
        let old = json!({"KSampler": {"input": {"required": {"sampler_name": [["euler", "dpmpp_2m"], {}]}}}});
        let new = json!({"KSampler": {"input": {"required": {"sampler_name": ["COMBO", {"options": ["euler"]}]}}}});
        assert_eq!(combo_choices(&old, "KSampler", "sampler_name"), ["euler", "dpmpp_2m"]);
        assert_eq!(combo_choices(&new, "KSampler", "sampler_name"), ["euler"]);
        assert!(combo_choices(&new, "KSampler", "scheduler").is_empty());
    }

    #[test]
    fn explains_refused_workflows_per_node() {
        let body = json!({
            "error": {"message": "Prompt outputs failed validation"},
            "node_errors": {"4": {"class_type": "CheckpointLoaderSimple",
                "errors": [{"message": "Value not in list", "details": "ckpt_name: 'gone.safetensors'"}]}}
        });
        let error = queue_error(reqwest::StatusCode::BAD_REQUEST, &body.to_string()).to_string();
        assert!(error.contains("Prompt outputs failed validation"), "{error}");
        assert!(error.contains("CheckpointLoaderSimple #4: Value not in list ckpt_name: 'gone.safetensors'"), "{error}");
    }

    #[tokio::test]
    async fn queues_waits_and_downloads_the_saved_picture() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/prompt"))
            .respond_with(|req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                assert_eq!(body["prompt"]["6"]["inputs"]["text"], "a \"quoted\" cat");
                ResponseTemplate::new(200).set_body_json(json!({"prompt_id": "abc", "number": 1, "node_errors": {}}))
            })
            .mount(&server)
            .await;
        // First poll: still queued (empty history); second: done.
        Mock::given(path("/history/abc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/history/abc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"abc": {
                "status": {"status_str": "success", "completed": true, "messages": []},
                "outputs": {"9": {"images": [
                    {"filename": "LocalTavern_00001_.png", "subfolder": "", "type": "output"}
                ]}}
            }})))
            .mount(&server)
            .await;
        Mock::given(path("/view"))
            .and(query_param("filename", "LocalTavern_00001_.png"))
            .and(query_param("type", "output"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(PNG))
            .mount(&server)
            .await;

        let image = generate(&super::super::client(), &server.uri(), "", &request(), false).await.unwrap();
        assert_eq!(image.data, PNG);
        assert_eq!(image.mime_type, "image/png");
    }

    #[tokio::test]
    async fn freeing_vram_asks_comfyui_to_unload_its_models() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/free"))
            .and(wiremock::matchers::body_json(json!({"unload_models": true})))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        free_vram(&super::super::client(), &server.uri()).await;
    }

    #[tokio::test]
    async fn reports_execution_errors_from_history() {
        let server = MockServer::start().await;
        Mock::given(path("/prompt"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"prompt_id": "x"})))
            .mount(&server)
            .await;
        Mock::given(path("/history/x"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"x": {
                "status": {"status_str": "error", "completed": false, "messages": [
                    ["execution_start", {}],
                    ["execution_error", {"exception_message": "CUDA out of memory\n"}]
                ]},
                "outputs": {}
            }})))
            .mount(&server)
            .await;
        let error = generate(&super::super::client(), &server.uri(), "", &request(), false).await.unwrap_err().to_string();
        assert_eq!(error, "ComfyUI failed: CUDA out of memory");
    }

    #[tokio::test]
    async fn picks_the_first_checkpoint_when_none_is_chosen() {
        let server = MockServer::start().await;
        Mock::given(path("/object_info/CheckpointLoaderSimple"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"CheckpointLoaderSimple": {"input": {"required": {
                "ckpt_name": [["first.safetensors", "second.safetensors"], {}]}}}})))
            .mount(&server)
            .await;
        Mock::given(path("/object_info/KSampler"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(path("/prompt"))
            .respond_with(|req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                assert_eq!(body["prompt"]["4"]["inputs"]["ckpt_name"], "first.safetensors");
                // Refuse after checking, so the test ends here.
                ResponseTemplate::new(400).set_body_json(json!({"error": {"message": "stop here"}}))
            })
            .mount(&server)
            .await;
        let error = generate(&super::super::client(), &server.uri(), "", &ImageRequest { model: String::new(), ..request() }, false)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("stop here"), "{error}");
    }

    #[test]
    fn the_default_video_workflow_fills_wan_model_names() {
        let w = fill_workflow_with(
            DEFAULT_VIDEO_WORKFLOW,
            &request(),
            &[
                ("%unet%", "wan2.1_t2v_1.3B_fp16.safetensors"),
                ("%clip%", "umt5_xxl_fp8_e4m3fn_scaled.safetensors"),
                ("%vae%", "wan_2.1_vae.safetensors"),
            ],
        )
        .unwrap();
        assert_eq!(w["37"]["inputs"]["unet_name"], "wan2.1_t2v_1.3B_fp16.safetensors");
        assert_eq!(w["38"]["inputs"]["clip_name"], "umt5_xxl_fp8_e4m3fn_scaled.safetensors");
        assert_eq!(w["38"]["inputs"]["type"], "wan");
        assert_eq!(w["39"]["inputs"]["vae_name"], "wan_2.1_vae.safetensors");
        assert_eq!(w["3"]["inputs"]["seed"], 42);
        assert_eq!(w["40"]["inputs"]["width"], 832);
        assert_eq!(w["40"]["inputs"]["length"], 33);
        assert_eq!(w["47"]["class_type"], "SaveWEBM");
        assert!(w["6"]["inputs"]["text"].as_str().unwrap().contains("a \"quoted\" cat"));
    }

    #[test]
    fn prefers_a_small_wan_text_to_video_model() {
        let names = vec![
            "wan2.1_i2v_480p_14B_fp16.safetensors".into(),
            "wan2.1_t2v_14B_fp16.safetensors".into(),
            "wan2.1_t2v_1.3B_fp16.safetensors".into(),
            "sdxl.safetensors".into(),
        ];
        assert_eq!(rank_pick(&names, unet_rank), Some("wan2.1_t2v_1.3B_fp16.safetensors"));
        let clips = vec!["clip_l.safetensors".into(), "umt5_xxl_fp16.safetensors".into(), "umt5_xxl_fp8_e4m3fn_scaled.safetensors".into()];
        assert_eq!(rank_pick(&clips, clip_rank), Some("umt5_xxl_fp8_e4m3fn_scaled.safetensors"));
        let vaes = vec!["pixel_space".into(), "ae.safetensors".into(), "wan_2.1_vae.safetensors".into()];
        assert_eq!(rank_pick(&vaes, vae_rank), Some("wan_2.1_vae.safetensors"));
    }

    #[tokio::test]
    async fn the_built_in_video_workflow_uses_installed_wan_models() {
        let server = MockServer::start().await;
        Mock::given(path("/object_info/UNETLoader"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"UNETLoader": {"input": {"required": {
                "unet_name": [["wan2.1_i2v_14B.safetensors", "wan2.1_t2v_1.3B_fp16.safetensors"], {}]
            }}}})))
            .mount(&server)
            .await;
        Mock::given(path("/object_info/CLIPLoader"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"CLIPLoader": {"input": {"required": {
                "clip_name": [["clip_l.safetensors", "umt5_xxl_fp8_e4m3fn_scaled.safetensors"], {}]
            }}}})))
            .mount(&server)
            .await;
        Mock::given(path("/object_info/VAELoader"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"VAELoader": {"input": {"required": {
                "vae_name": [["pixel_space", "wan_2.1_vae.safetensors"], {}]
            }}}})))
            .mount(&server)
            .await;
        Mock::given(path("/prompt"))
            .respond_with(|req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                assert_eq!(body["prompt"]["37"]["inputs"]["unet_name"], "wan2.1_t2v_1.3B_fp16.safetensors");
                assert_eq!(body["prompt"]["38"]["inputs"]["clip_name"], "umt5_xxl_fp8_e4m3fn_scaled.safetensors");
                assert_eq!(body["prompt"]["39"]["inputs"]["vae_name"], "wan_2.1_vae.safetensors");
                assert_eq!(body["prompt"]["47"]["class_type"], "SaveWEBM");
                ResponseTemplate::new(400).set_body_json(json!({"error": {"message": "stop here"}}))
            })
            .mount(&server)
            .await;
        let error = generate(&super::super::client(), &server.uri(), "", &request(), true).await.unwrap_err().to_string();
        assert!(error.contains("stop here"), "{error}");
    }

    #[tokio::test]
    async fn a_clip_without_wan_models_is_drawn_as_a_still() {
        let server = MockServer::start().await;
        for node in ["UNETLoader", "CLIPLoader", "VAELoader"] {
            Mock::given(path(format!("/object_info/{node}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .mount(&server)
                .await;
        }
        Mock::given(path("/prompt"))
            .respond_with(|req: &Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                assert_eq!(body["prompt"]["4"]["class_type"], "CheckpointLoaderSimple");
                assert_eq!(body["prompt"]["4"]["inputs"]["ckpt_name"], "sdxl.safetensors");
                assert_eq!(body["prompt"]["9"]["class_type"], "SaveImage");
                ResponseTemplate::new(400).set_body_json(json!({"error": {"message": "stop here"}}))
            })
            .mount(&server)
            .await;
        let error = generate(&super::super::client(), &server.uri(), "", &request(), true).await.unwrap_err().to_string();
        assert!(error.contains("stop here"), "{error}");
    }
}
