//! Pictures drawn on the phone itself, by stable-diffusion.cpp's `sd-server`. The APK carries it as
//! `libsd-server.so` next to llama-server (see `scripts/build-sd-android.sh`); checkpoints live in the
//! app's `image-models` folder, apart from the chat models. The server loads one checkpoint, speaks
//! the A1111 API (so [`super::a1111`] talks to it), and is stopped again when it has been idle a
//! while - or right after each picture when it shares the phone's memory with the chat model.
//!
//! A phone CPU needs every shortcut to draw in about a minute rather than a quarter of an hour:
//! few-step models (LCM, Turbo) at CFG 1, flash attention, an SD 1.5 sized picture, and TAESD - a
//! tiny decoder that turns the result into pixels in seconds instead of minutes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::process::Child;

use super::{BackendOptions, ImageRequest};
use crate::error::{Error, Result};
use crate::providers::ondevice::server::{free_port, Launch};
use crate::providers::ondevice::Runtime;

pub const SERVER_FILE: &str = "libsd-server.so";
/// Overrides where the server binary is, e.g. a desktop build of sd-server for trying this out.
const SERVER_ENV: &str = "LOCALTAVERN_SD_SERVER";
/// Loading a checkpoint of a few gigabytes from phone storage takes a while.
const READY_TIMEOUT: Duration = Duration::from_secs(300);
/// Kept loaded this long after a picture, so the next one doesn't wait for the checkpoint again.
const IDLE_STOP: Duration = Duration::from_secs(300);

/// The tiny SD 1.x decoder, fetched next to the first checkpoint.
pub const TAESD_FILE: &str = "taesd.safetensors";
pub const TAESD_URL: &str = "https://huggingface.co/madebyollin/taesd/resolve/main/diffusion_pytorch_model.safetensors";
/// SD 1.5's own size budget; bigger pictures take far longer and come out with doubled bodies.
const MAX_PIXELS: u32 = 512 * 768;

/// sd.cpp's sampler names (it doesn't know A1111's), the ones that suit SD 1.5 and SDXL.
pub const SAMPLERS: [&str; 12] =
    ["euler_a", "euler", "heun", "dpm2", "dpm++2s_a", "dpm++2m", "dpm++2mv2", "ipndm", "ipndm_v", "lcm", "ddim_trailing", "tcd"];
pub const SCHEDULERS: [&str; 9] =
    ["discrete", "karras", "exponential", "ays", "gits", "sgm_uniform", "simple", "smoothstep", "lcm"];

/// A checkpoint in the image models folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageModelFile {
    pub file_name: String,
    pub size_bytes: u64,
}

/// The folder's checkpoints, sorted by name. Unfinished downloads (`.part`) are left out.
pub fn model_files(dir: &Path) -> Vec<ImageModelFile> {
    let mut files: Vec<ImageModelFile> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let lower = file_name.to_ascii_lowercase();
            if !(lower.ends_with(".gguf") || lower.ends_with(".safetensors")) || lower == TAESD_FILE {
                return None;
            }
            let size_bytes = entry.metadata().ok().filter(|m| m.is_file())?.len();
            Some(ImageModelFile { file_name, size_bytes })
        })
        .collect();
    files.sort_by_key(|f| f.file_name.to_lowercase());
    files
}

/// TAESD only fits SD 1.x; an SDXL checkpoint (its name says so: sdxl, pony...XL, illustriousXL)
/// would come out as noise with it, so those keep their own decoder.
pub fn uses_taesd(model: &Path) -> bool {
    let name = model.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    !name.contains("xl")
}

pub fn args(model: &Path, port: u16, threads: i32, taesd: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--model".into(),
        model.to_string_lossy().into_owned(),
        "--listen-ip".into(),
        "127.0.0.1".into(),
        "--listen-port".into(),
        port.to_string(),
        "--threads".into(),
        threads.max(1).to_string(),
        // Decodes the picture in tiles: the full-size decode alone can need gigabytes.
        "--vae-tiling".into(),
        // A fifth faster on a phone, and less memory.
        "--diffusion-fa".into(),
    ];
    if let Some(taesd) = taesd {
        args.extend(["--taesd".into(), taesd.to_string_lossy().into_owned()]);
    }
    args
}

/// Fits a request to the checkpoint: LCM and Turbo models want few steps, CFG 1 and their own
/// sampler (the Settings values are meant for the PC servers), and every SD 1.x model gets an
/// SD 1.5 sized picture of the same shape.
pub fn tune(request: &mut ImageRequest, model: &Path) {
    let name = model.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.contains("lcm") {
        request.sampler = "lcm".into();
        request.steps = if request.steps > 8 { 5 } else { request.steps.max(2) };
        request.cfg_scale = 1.0;
    } else if name.contains("turbo") {
        request.sampler = "euler_a".into();
        request.steps = request.steps.clamp(1, 4);
        request.cfg_scale = 1.0;
    }
    let pixels = request.width * request.height;
    if pixels > MAX_PIXELS && !name.contains("xl") {
        let scale = (MAX_PIXELS as f64 / pixels as f64).sqrt();
        let fit = |side: u32| (((side as f64 * scale) / 64.0).round() as u32 * 64).max(256);
        (request.width, request.height) = (fit(request.width), fit(request.height));
    }
}

struct Running {
    model: PathBuf,
    taesd: bool,
    threads: i32,
    base_url: String,
    child: Child,
}

pub struct OnDeviceImages {
    dir: PathBuf,
    runtime: Option<Runtime>,
    running: tokio::sync::Mutex<Option<Running>>,
    /// Bumped by every picture, so a pending idle stop can tell it has been overtaken.
    uses: AtomicU64,
}

impl OnDeviceImages {
    pub fn new(dir: PathBuf) -> Self {
        Self::with_runtime(dir, Runtime::find_program(SERVER_ENV, SERVER_FILE))
    }

    pub fn with_runtime(dir: PathBuf, runtime: Option<Runtime>) -> Self {
        Self { dir, runtime, running: Default::default(), uses: AtomicU64::new(0) }
    }

    /// Whether this build carries the server (the Android app does).
    pub fn available(&self) -> bool {
        self.runtime.is_some()
    }

    pub fn models_dir(&self) -> &Path {
        &self.dir
    }

    /// The checkpoints for the Settings picker, with sd.cpp's samplers and schedulers.
    pub fn options(&self) -> BackendOptions {
        BackendOptions {
            models: model_files(&self.dir).into_iter().map(|f| f.file_name).collect(),
            samplers: SAMPLERS.map(String::from).to_vec(),
            schedulers: SCHEDULERS.map(String::from).to_vec(),
        }
    }

    /// The checkpoint `model` names, or the first one when it's empty.
    pub fn model_path(&self, model: &str) -> Result<PathBuf> {
        let files = model_files(&self.dir);
        let file = if model.is_empty() { files.first() } else { files.iter().find(|f| f.file_name == model) };
        match file {
            Some(file) => Ok(self.dir.join(&file.file_name)),
            None if model.is_empty() => {
                Err(Error::Other("There's no picture model on this phone yet. Download one in Settings → Pictures.".into()))
            }
            None => Err(Error::Other(format!("The picture model '{model}' isn't on this phone any more."))),
        }
    }

    /// The running server's address, starting it with `model` (or switching to it) first.
    pub async fn ensure(&self, client: &reqwest::Client, model: &str, threads: i32) -> Result<String> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| Error::Other("Drawing on the device isn't available in this build.".into()))?;
        let model = self.model_path(model)?;
        let taesd = Some(self.dir.join(TAESD_FILE)).filter(|t| t.is_file() && uses_taesd(&model));
        let mut slot = self.running.lock().await;
        if let Some(running) = slot.as_mut() {
            let same = running.model == model && running.threads == threads && running.taesd == taesd.is_some();
            if same && matches!(running.child.try_wait(), Ok(None)) {
                return Ok(running.base_url.clone());
            }
        }
        if let Some(mut old) = slot.take() {
            let _ = old.child.kill().await;
        }
        let port = free_port()?;
        let base_url = format!("http://127.0.0.1:{port}");
        let launch = Launch {
            binary: &runtime.binary,
            lib_dir: &runtime.lib_dir,
            args: args(&model, port, threads, taesd.as_deref()),
            ready_url: format!("{base_url}/sdapi/v1/options"),
            what: "on-device picture server",
        };
        let child = launch.run(client, READY_TIMEOUT).await?;
        *slot = Some(Running { model, taesd: taesd.is_some(), threads, base_url: base_url.clone(), child });
        Ok(base_url)
    }

    /// Stops the server now: a picture was stopped (it would keep drawing otherwise), memory is
    /// needed for the chat model, or the app is closing.
    pub async fn stop(&self) {
        self.uses.fetch_add(1, Ordering::SeqCst);
        if let Some(mut running) = self.running.lock().await.take() {
            let _ = running.child.kill().await;
        }
    }

    /// Stops the server once no picture has followed for a while.
    pub fn stop_when_idle(self: &Arc<Self>) {
        let ticket = self.uses.fetch_add(1, Ordering::SeqCst) + 1;
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(IDLE_STOP).await;
            if this.uses.load(Ordering::SeqCst) == ticket {
                this.stop().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in files {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn lists_checkpoints_and_skips_partial_downloads_and_other_files() {
        let dir = folder(&["b.safetensors", "A.gguf", "c.gguf.part", "notes.txt", "taesd.safetensors"]);
        let names: Vec<_> = model_files(dir.path()).into_iter().map(|f| f.file_name).collect();
        assert_eq!(names, ["A.gguf", "b.safetensors"]);
    }

    #[test]
    fn an_empty_choice_takes_the_first_checkpoint() {
        let dir = folder(&["b.safetensors", "a.gguf"]);
        let images = OnDeviceImages::with_runtime(dir.path().to_path_buf(), None);
        assert_eq!(images.model_path("").unwrap(), dir.path().join("a.gguf"));
        assert_eq!(images.model_path("b.safetensors").unwrap(), dir.path().join("b.safetensors"));
        assert!(images.model_path("gone.gguf").unwrap_err().to_string().contains("isn't on this phone"));

        let empty = folder(&[]);
        let none = OnDeviceImages::with_runtime(empty.path().to_path_buf(), None);
        assert!(none.model_path("").unwrap_err().to_string().contains("Download one"));
    }

    #[test]
    fn args_serve_on_localhost_and_add_the_fast_decoder_when_there_is_one() {
        let joined = args(Path::new("m/sd15.gguf"), 4321, 0, None).join(" ");
        assert_eq!(joined, "--model m/sd15.gguf --listen-ip 127.0.0.1 --listen-port 4321 --threads 1 --vae-tiling --diffusion-fa");
        let fast = args(Path::new("m/sd15.gguf"), 4321, 4, Some(Path::new("m/taesd.safetensors"))).join(" ");
        assert!(fast.ends_with("--diffusion-fa --taesd m/taesd.safetensors"), "{fast}");
        assert!(uses_taesd(Path::new("m/anything_v30-LCM.q8_0.gguf")));
        assert!(!uses_taesd(Path::new("m/ponyDiffusionV6XL.gguf")));
    }

    fn request(width: u32, height: u32) -> ImageRequest {
        ImageRequest {
            prompt: "p".into(),
            negative_prompt: String::new(),
            width,
            height,
            steps: 25,
            cfg_scale: 7.0,
            seed: 1,
            model: String::new(),
            sampler: "euler_a".into(),
            scheduler: "discrete".into(),
        }
    }

    #[test]
    fn lcm_and_turbo_models_get_their_steps_and_every_picture_an_sd15_size() {
        let mut lcm = request(832, 1216);
        tune(&mut lcm, Path::new("m/anything_v30-LCM.q8_0.gguf"));
        assert_eq!((lcm.sampler.as_str(), lcm.steps, lcm.cfg_scale), ("lcm", 5, 1.0));
        assert_eq!((lcm.width, lcm.height), (512, 768), "same shape, SD 1.5 budget");

        let mut few = ImageRequest { steps: 4, ..request(512, 512) };
        tune(&mut few, Path::new("m/dreamshaper_8LCM-iq4_nl.gguf"));
        assert_eq!((few.steps, few.width, few.height), (4, 512, 512), "a small picture and few steps stay");

        let mut turbo = request(1024, 1024);
        tune(&mut turbo, Path::new("m/sd_turbo-f16-q8_0.gguf"));
        assert_eq!((turbo.sampler.as_str(), turbo.steps, turbo.cfg_scale), ("euler_a", 4, 1.0));
        assert_eq!((turbo.width, turbo.height), (640, 640));

        let mut plain = request(832, 1216);
        tune(&mut plain, Path::new("m/sd15.gguf"));
        assert_eq!((plain.sampler.as_str(), plain.steps, plain.cfg_scale), ("euler_a", 25, 7.0));

        let mut xl = request(832, 1216);
        tune(&mut xl, Path::new("m/ponyDiffusionV6XL.gguf"));
        assert_eq!((xl.width, xl.height), (832, 1216), "SDXL keeps its own size");
    }

    #[tokio::test]
    async fn without_the_server_the_build_says_so() {
        let dir = folder(&["a.gguf"]);
        let images = OnDeviceImages::with_runtime(dir.path().to_path_buf(), None);
        assert!(!images.available());
        let err = images.ensure(&reqwest::Client::new(), "", 4).await.unwrap_err();
        assert!(err.to_string().contains("isn't available"), "{err}");
    }
}
