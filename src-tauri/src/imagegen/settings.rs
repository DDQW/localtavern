//! Image generation settings, stored as one JSON value in `AppSettings`. Checkpoint, sampler and
//! scheduler are kept per backend because their names differ (A1111's "DPM++ 2M" is ComfyUI's
//! "dpmpp_2m"), so switching backends doesn't leave a value the other one rejects.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::repo::app_settings;

const SETTINGS_KEY: &str = "imagegen.settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ImageBackend {
    /// AUTOMATIC1111, Forge, reForge and SD.Next: they share the `/sdapi/v1` API.
    #[default]
    A1111,
    ComfyUi,
    /// Grok Imagine on xAI's servers, paid for by the SuperGrok sign-in (see `imagegen::grok`).
    Grok,
    /// stable-diffusion.cpp running on the phone itself (see `imagegen::ondevice`).
    OnDevice,
}

/// Whether the chat model writes the picture prompt as tags (SD 1.5 / SDXL / Pony / Illustrious)
/// or as plain sentences (FLUX and other natural-language models).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PromptStyle {
    #[default]
    Tags,
    Natural,
}

/// Where one backend lives and what it renders with. Empty strings mean "the server's default".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BackendConfig {
    pub url: String,
    pub model: String,
    pub sampler: String,
    pub scheduler: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageGenSettings {
    pub backend: ImageBackend,
    pub a1111: BackendConfig,
    pub comfy: BackendConfig,
    /// Only `url` (xAI's API) and `model` apply; Grok has no samplers or schedulers.
    pub grok: BackendConfig,
    /// `model` is a checkpoint's file name in the image models folder; `url` is unused (the server
    /// gets a free port each time it starts).
    pub on_device: BackendConfig,
    /// A ComfyUI workflow in API format with `%prompt%`-style placeholders; empty uses the built-in
    /// text-to-image workflow.
    pub comfy_workflow: String,
    /// ComfyUI API workflow for story videos (`[video: ...]`). Empty uses the built-in Wan
    /// text-to-video workflow. Same `%prompt%` placeholders as the still workflow.
    #[serde(default)]
    pub comfy_video_workflow: String,
    /// What starts ComfyUI (its `run_*.bat`, say); run hidden when LocalTavern starts. Empty: the
    /// user starts ComfyUI themselves.
    pub comfy_launcher: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg_scale: f32,
    /// -1 picks a new random seed every time.
    pub seed: i64,
    /// Put in front of every prompt (quality tags, a style).
    pub prompt_prefix: String,
    pub negative_prompt: String,
    pub prompt_style: PromptStyle,
    /// Show the model-written prompt for editing before anything is rendered.
    pub review_prompt: bool,
    /// Unload Ollama's model before rendering, so the two don't fight over one graphics card.
    pub unload_ollama_first: bool,
    /// Draw the photos characters send in the story (`[photo: ...]` tags) automatically.
    pub auto_photos: bool,
    /// Render the videos characters send in the story (`[video: ...]` tags) automatically.
    /// `bool` defaults to false, which would drop the instruction from settings saved before this
    /// switch existed, so the model would never be asked. Missing means on, same as a new install.
    #[serde(default = "default_true")]
    pub auto_videos: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ImageGenSettings {
    fn default() -> Self {
        Self {
            backend: ImageBackend::A1111,
            a1111: BackendConfig { url: "http://127.0.0.1:7860".into(), sampler: "Euler a".into(), ..Default::default() },
            comfy: BackendConfig {
                url: "http://127.0.0.1:8188".into(),
                sampler: "euler".into(),
                scheduler: "normal".into(),
                ..Default::default()
            },
            grok: BackendConfig { url: "https://api.x.ai/v1".into(), ..Default::default() },
            on_device: BackendConfig { sampler: "euler_a".into(), scheduler: "discrete".into(), ..Default::default() },
            comfy_workflow: String::new(),
            comfy_video_workflow: String::new(),
            comfy_launcher: String::new(),
            // SDXL's portrait bucket; the "face" and scene modes reshape it (see `prompts::size_for`).
            width: 832,
            height: 1216,
            steps: 25,
            cfg_scale: 7.0,
            seed: -1,
            prompt_prefix: "best quality, highly detailed".into(),
            negative_prompt: "lowres, bad anatomy, bad hands, extra fingers, blurry, watermark, text, signature".into(),
            prompt_style: PromptStyle::Tags,
            review_prompt: true,
            unload_ollama_first: true,
            auto_photos: true,
            auto_videos: true,
        }
    }
}

impl ImageGenSettings {
    pub fn active(&self) -> &BackendConfig {
        match self.backend {
            ImageBackend::A1111 => &self.a1111,
            ImageBackend::ComfyUi => &self.comfy,
            ImageBackend::Grok => &self.grok,
            ImageBackend::OnDevice => &self.on_device,
        }
    }

    /// Clamps values a backend would reject (zero steps, sizes that aren't multiples of 8).
    pub fn sanitized(mut self) -> Self {
        let defaults = Self::default();
        let size = |v: u32, fallback: u32| if v < 64 { fallback } else { (v.min(4096) / 8) * 8 };
        self.width = size(self.width, defaults.width);
        self.height = size(self.height, defaults.height);
        self.steps = self.steps.clamp(1, 150);
        if !self.cfg_scale.is_finite() || self.cfg_scale <= 0.0 {
            self.cfg_scale = defaults.cfg_scale;
        }
        self.cfg_scale = self.cfg_scale.min(30.0);
        if self.seed < -1 {
            self.seed = -1;
        }
        for (config, fallback) in
            [(&mut self.a1111, &defaults.a1111), (&mut self.comfy, &defaults.comfy), (&mut self.grok, &defaults.grok)]
        {
            config.url = config.url.trim().trim_end_matches('/').to_string();
            if config.url.is_empty() {
                config.url = fallback.url.clone();
            }
        }
        // Explorer's "Copy as path" wraps the path in quotes.
        self.comfy_launcher = self.comfy_launcher.trim().trim_matches('"').trim().to_string();
        // Grok reads sentences; tags come out as a jumble of everything at once.
        if self.backend == ImageBackend::Grok {
            self.prompt_style = PromptStyle::Natural;
        }
        self
    }
}

pub fn load(conn: &Connection) -> Result<ImageGenSettings> {
    let Some(json) = app_settings::get(conn, SETTINGS_KEY)?.filter(|j| !j.is_empty()) else {
        return Ok(ImageGenSettings::default());
    };
    // A value we can't read falls back to defaults rather than breaking every picture.
    Ok(serde_json::from_str::<ImageGenSettings>(&json).unwrap_or_default().sanitized())
}

pub fn save(conn: &Connection, settings: ImageGenSettings) -> Result<ImageGenSettings> {
    let settings = settings.sanitized();
    app_settings::set(conn, SETTINGS_KEY, &serde_json::to_string(&settings)?)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn defaults_when_nothing_or_garbage_is_stored() {
        let conn = open_in_memory().unwrap();
        assert_eq!(load(&conn).unwrap(), ImageGenSettings::default());
        app_settings::set(&conn, SETTINGS_KEY, "{nope").unwrap();
        assert_eq!(load(&conn).unwrap(), ImageGenSettings::default());
    }

    #[test]
    fn round_trips_and_fills_missing_fields_with_defaults() {
        let conn = open_in_memory().unwrap();
        let mut settings = ImageGenSettings { backend: ImageBackend::ComfyUi, steps: 30, ..Default::default() };
        settings.comfy.model = "flux.safetensors".into();
        save(&conn, settings.clone()).unwrap();
        assert_eq!(load(&conn).unwrap(), settings);

        app_settings::set(&conn, SETTINGS_KEY, r#"{"backend":"comfyUi","steps":12}"#).unwrap();
        let partial = load(&conn).unwrap();
        assert_eq!((partial.backend, partial.steps), (ImageBackend::ComfyUi, 12));
        assert_eq!(partial.comfy.url, "http://127.0.0.1:8188");

        // Saved before story videos existed: the model is still asked. An explicit off stays off.
        app_settings::set(&conn, SETTINGS_KEY, r#"{"autoPhotos":true}"#).unwrap();
        let legacy = load(&conn).unwrap();
        assert!(legacy.auto_photos && legacy.auto_videos);
        app_settings::set(&conn, SETTINGS_KEY, r#"{"autoVideos":false}"#).unwrap();
        assert!(!load(&conn).unwrap().auto_videos);
    }

    #[test]
    fn sanitizing_fixes_values_a_backend_would_reject() {
        let mut s = ImageGenSettings { width: 1000, height: 10, steps: 0, cfg_scale: f32::NAN, seed: -7, ..Default::default() };
        s.a1111.url = " http://box:7860/ ".into();
        s.comfy.url = "  ".into();
        s.comfy_launcher = " \"D:\\AI\\ComfyUI\\run_amd_gpu.bat\" ".into();
        let s = s.sanitized();
        assert_eq!(s.comfy_launcher, "D:\\AI\\ComfyUI\\run_amd_gpu.bat");
        assert_eq!((s.width, s.height, s.steps, s.cfg_scale, s.seed), (1000, 1216, 1, 7.0, -1));
        assert_eq!(s.a1111.url, "http://box:7860");
        assert_eq!(s.comfy.url, "http://127.0.0.1:8188");
        assert_eq!(ImageGenSettings { width: 1001, ..Default::default() }.sanitized().width, 1000);
    }

    #[test]
    fn grok_always_gets_sentences_and_keeps_its_api_address() {
        let s = ImageGenSettings { backend: ImageBackend::Grok, prompt_style: PromptStyle::Tags, ..Default::default() }.sanitized();
        assert_eq!(s.prompt_style, PromptStyle::Natural);
        assert_eq!(s.active().url, "https://api.x.ai/v1");

        // Settings saved before Grok existed get its defaults.
        let conn = open_in_memory().unwrap();
        app_settings::set(&conn, SETTINGS_KEY, r#"{"backend":"grok"}"#).unwrap();
        assert_eq!(load(&conn).unwrap().grok.url, "https://api.x.ai/v1");
    }
}
