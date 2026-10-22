//! ComfyUI: queue a workflow (`POST /prompt`), wait for it in `/history`, fetch the picture from
//! `/view`. The built-in workflow is plain checkpoint text-to-image; a custom one (exported with
//! "Export (API)") can use the same `%placeholder%` names SillyTavern uses.

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

/// Fills a workflow's placeholders. A string that is exactly a numeric placeholder (`"%seed%"`)
/// becomes a JSON number; placeholders inside longer strings are replaced as text.
pub fn fill_workflow(workflow: &str, request: &ImageRequest) -> Result<Value> {
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
    let texts: [(&str, &str); 5] = [
        ("%negative_prompt%", &request.negative_prompt),
        ("%prompt%", &request.prompt),
        ("%model%", &request.model),
        ("%sampler%", &request.sampler),
        ("%scheduler%", &request.scheduler),
    ];

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

pub async fn generate(client: &reqwest::Client, url: &str, custom_workflow: &str, request: &ImageRequest) -> Result<GeneratedImage> {
    let template = if custom_workflow.trim().is_empty() { DEFAULT_WORKFLOW } else { custom_workflow };
    let mut request = request.clone();
    if request.model.is_empty() && template.contains("%model%") {
        // Nothing picked yet: use the first checkpoint ComfyUI knows about.
        request.model = options(client, url).await?.models.into_iter().next().ok_or_else(|| {
            Error::Other(format!("{SERVER} has no checkpoints - put one in ComfyUI's models/checkpoints folder."))
        })?;
    }
    let workflow = fill_workflow(template, &request)?;

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

        let image = generate(&super::super::client(), &server.uri(), "", &request()).await.unwrap();
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
        let error = generate(&super::super::client(), &server.uri(), "", &request()).await.unwrap_err().to_string();
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
        let error = generate(&super::super::client(), &server.uri(), "", &ImageRequest { model: String::new(), ..request() })
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("stop here"), "{error}");
    }
}
