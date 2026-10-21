//! Models that run on this device, through llama.cpp's `llama-server`. The APK carries the server
//! as `libllama-server.so` (Android only lets an app execute files from its native library folder);
//! the GGUF models live in the app's `models` folder. The server is started for whichever model a
//! reply asks for and stopped again once it has sat idle for a while, giving back its memory.
//!
//! A model sees pictures when a vision projector (an `mmproj` GGUF in the same folder) is picked for
//! it in Settings.

pub mod download;
pub mod server;
pub mod wire;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use self::server::{Server, ServerSpec};
use super::error::ProviderError;
use super::http::{ensure_success, send};
use super::{openai_compat, ChunkStream, LlmProvider};
use crate::llm::LlmRequest;

pub const PROVIDER_ID: &str = "ondevice";
const NAME: &str = "On-device model";
/// Overrides where the server binary is, e.g. a desktop build of llama-server for trying this out.
const SERVER_ENV: &str = "LOCALTAVERN_LLAMA_SERVER";
const SERVER_FILE: &str = "libllama-server.so";
/// Loading a model of several gigabytes from phone storage takes a while.
const READY_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OnDeviceOptions {
    pub context_size: i32,
    /// CPU threads. A phone's few big cores do the work; using the small ones as well slows it down.
    pub threads: i32,
    /// How long the model stays loaded after the last reply. 0 = keep it loaded.
    pub idle_stop_secs: u64,
    /// The vision projector file each model sees pictures through, by model name.
    pub projectors: BTreeMap<String, String>,
}

impl Default for OnDeviceOptions {
    fn default() -> Self {
        Self { context_size: 8192, threads: 4, idle_stop_secs: 600, projectors: BTreeMap::new() }
    }
}

/// Where the provider reads the current options from (persisted, user-editable settings in the app).
pub trait OnDeviceOptionsSource: Send + Sync {
    fn current(&self) -> OnDeviceOptions;
}

impl OnDeviceOptionsSource for OnDeviceOptions {
    fn current(&self) -> OnDeviceOptions {
        self.clone()
    }
}

/// A GGUF file in the models folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFile {
    /// The file name without `.gguf` - what the chat's model picker shows.
    pub name: String,
    pub file_name: String,
    pub size_bytes: u64,
    /// A vision projector rather than a model.
    pub projector: bool,
}

/// The models folder's GGUF files, sorted by name. Unfinished downloads (`.part`) are left out.
pub fn model_files(dir: &Path) -> Vec<ModelFile> {
    let mut files: Vec<ModelFile> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let name = file_name.strip_suffix(".gguf")?.to_string();
            let size_bytes = entry.metadata().ok().filter(|m| m.is_file())?.len();
            let projector = file_name.to_ascii_lowercase().contains("mmproj");
            Some(ModelFile { name, file_name, size_bytes, projector })
        })
        .collect();
    files.sort_by_key(|f| f.name.to_lowercase());
    files
}

/// The executable and the folder of libraries it loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub binary: PathBuf,
    pub lib_dir: PathBuf,
}

impl Runtime {
    /// The bundled server, or the one [`SERVER_ENV`] names. `None` when this build has neither.
    pub fn find() -> Option<Runtime> {
        let binary = match std::env::var_os(SERVER_ENV).filter(|v| !v.is_empty()) {
            Some(path) => PathBuf::from(path),
            None => native_lib_dir()?.join(SERVER_FILE),
        };
        let lib_dir = binary.parent()?.to_path_buf();
        binary.is_file().then_some(Runtime { binary, lib_dir })
    }
}

/// The folder this app's own native library was loaded from - on Android, the app's native library
/// folder, where the bundled server sits next to it.
fn native_lib_dir() -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    let line = maps.lines().find(|l| l.ends_with("/liblocaltavern_lib.so"))?;
    let path = &line[line.find('/')?..];
    Path::new(path).parent().map(Path::to_path_buf)
}

/// Replies in progress and when the last one ended, for the idle stop.
#[derive(Default)]
struct Activity {
    active: AtomicUsize,
    last_end: Mutex<Option<Instant>>,
}

/// Held by a reply's stream; dropping it (the reply ended, failed or was cancelled) marks the end.
struct ActivityGuard(Arc<Activity>);

impl ActivityGuard {
    fn new(activity: Arc<Activity>) -> Self {
        activity.active.fetch_add(1, Ordering::SeqCst);
        Self(activity)
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        *self.0.last_end.lock().unwrap() = Some(Instant::now());
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct OnDeviceProvider {
    client: reqwest::Client,
    models_dir: PathBuf,
    runtime: Option<Runtime>,
    options: Arc<dyn OnDeviceOptionsSource>,
    server: Arc<tokio::sync::Mutex<Option<Server>>>,
    activity: Arc<Activity>,
    watching: AtomicBool,
}

impl OnDeviceProvider {
    pub fn new(
        client: reqwest::Client,
        models_dir: PathBuf,
        runtime: Option<Runtime>,
        options: Arc<dyn OnDeviceOptionsSource>,
    ) -> Self {
        Self {
            client,
            models_dir,
            runtime,
            options,
            server: Default::default(),
            activity: Default::default(),
            watching: AtomicBool::new(false),
        }
    }

    /// Whether this build can run models at all.
    pub fn available(&self) -> bool {
        self.runtime.is_some()
    }

    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    /// The model the server currently has loaded.
    pub async fn loaded_model(&self) -> Option<String> {
        let mut server = self.server.lock().await;
        let server = server.as_mut()?;
        server.is_alive().then(|| model_name(&server.spec.model))
    }

    /// Stops the server (app exit, a model being deleted, or Settings changing how it runs).
    pub async fn shutdown(&self) {
        if let Some(server) = self.server.lock().await.take() {
            server.stop().await;
        }
    }

    fn spec(&self, model: &str) -> Result<ServerSpec, ProviderError> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| ProviderError::invalid_request(format!("{NAME}s aren't available in this build.")))?;
        let files = model_files(&self.models_dir);
        let wanted = files
            .iter()
            .filter(|f| !f.projector)
            .find(|f| f.name == model)
            .or_else(|| if model.is_empty() { files.iter().find(|f| !f.projector) } else { None })
            .ok_or_else(|| {
                ProviderError::invalid_request(if model.is_empty() {
                    "There's no model on this device yet. Download one in Settings.".to_string()
                } else {
                    format!("The model '{model}' isn't on this device any more.")
                })
            })?;

        let options = self.options.current();
        let projector = options
            .projectors
            .get(&wanted.name)
            .map(|file| self.models_dir.join(file))
            .filter(|path| path.is_file());
        Ok(ServerSpec {
            binary: runtime.binary.clone(),
            lib_dir: runtime.lib_dir.clone(),
            model: self.models_dir.join(&wanted.file_name),
            projector,
            context_size: options.context_size.max(512),
            threads: options.threads.max(1),
        })
    }

    /// The running server's address, starting it (or switching it to `spec`) first.
    async fn ensure_server(&self, spec: ServerSpec) -> Result<String, ProviderError> {
        let mut slot = self.server.lock().await;
        if let Some(server) = slot.as_mut() {
            if server.spec == spec && server.is_alive() {
                return Ok(server.base_url.clone());
            }
        }
        if let Some(old) = slot.take() {
            old.stop().await;
        }
        let server = Server::start(spec, &self.client, READY_TIMEOUT).await?;
        let base_url = server.base_url.clone();
        *slot = Some(server);
        drop(slot);
        self.watch_idle();
        Ok(base_url)
    }

    /// Starts (once) the loop that stops the server after `idle_stop_secs` without a reply.
    fn watch_idle(&self) {
        if self.watching.swap(true, Ordering::SeqCst) {
            return;
        }
        let (server, activity, options) = (self.server.clone(), self.activity.clone(), self.options.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                let idle = options.current().idle_stop_secs;
                if idle == 0 || activity.active.load(Ordering::SeqCst) > 0 {
                    continue;
                }
                let ended = *activity.last_end.lock().unwrap();
                if ended.is_some_and(|t| t.elapsed() >= Duration::from_secs(idle)) {
                    if let Some(server) = server.lock().await.take() {
                        server.stop().await;
                    }
                }
            }
        });
    }
}

fn model_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

#[async_trait]
impl LlmProvider for OnDeviceProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        Ok(model_files(&self.models_dir).into_iter().filter(|f| !f.projector).map(|f| f.name).collect())
    }

    /// The server only gives a conversation the context size it was started with.
    async fn context_window(&self, _model: &str) -> Option<i32> {
        Some(self.options.current().context_size)
    }

    async fn stream_completion(&self, request: LlmRequest) -> Result<ChunkStream, ProviderError> {
        let spec = self.spec(&request.model)?;
        let sees_pictures = spec.projector.is_some();
        let has_pictures = request.history.iter().any(|m| m.attachments.as_ref().is_some_and(|a| !a.is_empty()));
        if has_pictures && !sees_pictures {
            return Err(ProviderError::invalid_request(format!(
                "'{}' can't see pictures. Pick a vision projector for it in Settings → {NAME}s, or remove the picture.",
                model_name(&spec.model)
            )));
        }

        let guard = ActivityGuard::new(self.activity.clone());
        let base_url = self.ensure_server(spec).await?;
        let json = serde_json::to_string(&wire::to_request(&request))
            .map_err(|e| ProviderError::unknown(format!("Failed to serialize the on-device request: {e}")))?;
        let http_request = self
            .client
            .post(format!("{base_url}/v1/chat/completions"))
            .header("content-type", "application/json")
            .body(json);
        let response = send(http_request, NAME).await?;
        let response = ensure_success(response, to_provider_error).await?;
        Ok(openai_compat::read_chunks(response.bytes_stream(), NAME)
            .map(move |chunk| {
                let _held = &guard;
                chunk
            })
            .boxed())
    }
}

/// llama-server errors: `{"error":{"code":400,"message":"...","type":"invalid_request_error"}}`.
fn to_provider_error(status: reqwest::StatusCode, body: &str) -> ProviderError {
    #[derive(Deserialize)]
    struct Body {
        error: Option<Detail>,
    }
    #[derive(Deserialize)]
    struct Detail {
        message: Option<String>,
    }
    let message = serde_json::from_str::<Body>(body)
        .ok()
        .and_then(|b| b.error?.message)
        .unwrap_or_else(|| format!("{NAME} error {}", super::http::describe_status(status)));
    let mut error = if status.is_client_error() {
        ProviderError::invalid_request(message)
    } else {
        ProviderError::unknown(message)
    };
    error.http_status = Some(status.as_u16());
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmAttachment, LlmMessage, LlmRole};
    use crate::providers::error::ProviderErrorKind;

    fn folder(files: &[(&str, usize)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, size) in files {
            std::fs::write(dir.path().join(name), vec![0u8; *size]).unwrap();
        }
        dir
    }

    fn provider(dir: &Path, options: OnDeviceOptions) -> OnDeviceProvider {
        let runtime = Runtime { binary: dir.join(SERVER_FILE), lib_dir: dir.to_path_buf() };
        OnDeviceProvider::new(reqwest::Client::new(), dir.to_path_buf(), Some(runtime), Arc::new(options))
    }

    #[test]
    fn lists_ggufs_by_name_marks_projectors_and_skips_partial_downloads() {
        let dir = folder(&[("b-model.gguf", 3), ("A-model.gguf", 2), ("mmproj-x.gguf", 1), ("c.gguf.part", 5), ("notes.txt", 1)]);
        let files = model_files(dir.path());
        let names: Vec<_> = files.iter().map(|f| (f.name.as_str(), f.projector, f.size_bytes)).collect();
        assert_eq!(names, [("A-model", false, 2), ("b-model", false, 3), ("mmproj-x", true, 1)]);
    }

    #[tokio::test]
    async fn the_model_list_leaves_projectors_out() {
        let dir = folder(&[("Luchador.gguf", 1), ("mmproj-gemma.gguf", 1)]);
        assert_eq!(provider(dir.path(), Default::default()).list_models().await.unwrap(), ["Luchador"]);
    }

    #[test]
    fn the_spec_uses_the_picked_projector_and_falls_back_to_the_first_model() {
        let dir = folder(&[("Luchador.gguf", 1), ("Mag-Mell.gguf", 1), ("mmproj-gemma.gguf", 1)]);
        let mut options = OnDeviceOptions { context_size: 4096, threads: 3, ..Default::default() };
        options.projectors.insert("Luchador".into(), "mmproj-gemma.gguf".into());
        let p = provider(dir.path(), options);

        let spec = p.spec("Luchador").unwrap();
        assert_eq!(spec.model, dir.path().join("Luchador.gguf"));
        assert_eq!(spec.projector, Some(dir.path().join("mmproj-gemma.gguf")));
        assert_eq!((spec.context_size, spec.threads), (4096, 3));

        assert_eq!(p.spec("Mag-Mell").unwrap().projector, None);
        assert_eq!(p.spec("").unwrap().model, dir.path().join("Luchador.gguf"));
        assert!(p.spec("Gone").unwrap_err().message.contains("isn't on this device"));
    }

    #[test]
    fn no_models_or_no_runtime_are_clear_errors() {
        let dir = folder(&[]);
        assert!(provider(dir.path(), Default::default()).spec("").unwrap_err().message.contains("Download one in Settings"));

        let without = OnDeviceProvider::new(reqwest::Client::new(), dir.path().into(), None, Arc::new(OnDeviceOptions::default()));
        assert!(!without.available());
        assert!(without.spec("x").unwrap_err().message.contains("aren't available"));
    }

    #[tokio::test]
    async fn a_picture_for_a_model_without_a_projector_is_refused_before_starting_anything() {
        let dir = folder(&[("Mag-Mell.gguf", 1), ("pic.png", 1)]);
        let mut message = LlmMessage::new(LlmRole::User, "Look");
        message.attachments = Some(vec![LlmAttachment {
            image_path: dir.path().join("pic.png").to_string_lossy().into_owned(),
            mime_type: "image/png".into(),
        }]);
        let request = LlmRequest {
            model: "Mag-Mell".into(),
            system_instruction: None,
            history: vec![message],
            generation_config: Default::default(),
            enable_web_search: None,
        };
        let err = provider(dir.path(), Default::default()).stream_completion(request).await.err().unwrap();
        assert_eq!(err.kind, ProviderErrorKind::InvalidRequest);
        assert!(err.message.contains("can't see pictures"), "{}", err.message);
    }

    #[test]
    fn server_errors_keep_their_message() {
        let body = r#"{"error":{"code":400,"message":"the request exceeds the available context size","type":"invalid_request_error"}}"#;
        let err = to_provider_error(reqwest::StatusCode::BAD_REQUEST, body);
        assert_eq!(err.kind, ProviderErrorKind::InvalidRequest);
        assert_eq!(err.message, "the request exceeds the available context size");
        assert_eq!(err.http_status, Some(400));
        assert!(to_provider_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "boom").message.contains("500"));
    }
}
