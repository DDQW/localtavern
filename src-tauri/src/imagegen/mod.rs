//! Picture generation through a local Stable Diffusion server, SillyTavern-style: the chat model
//! describes the character or scene (see [`prompts`]) and the server renders it. Two server
//! families are supported: the A1111 API (AUTOMATIC1111, Forge, reForge, SD.Next) and ComfyUI -
//! plus Grok Imagine in the cloud, through the SuperGrok sign-in, and stable-diffusion.cpp on the
//! phone itself.

pub mod a1111;
pub mod comfy;
pub mod grok;
pub mod launcher;
pub mod ollama;
pub mod ondevice;
pub mod prompts;
pub mod settings;

use std::time::Duration;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::providers::auth::AccessTokenSource;
pub use settings::{BackendConfig, ImageBackend, ImageGenSettings, PromptStyle};

/// One picture to render, with every setting already resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRequest {
    pub prompt: String,
    pub negative_prompt: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg_scale: f32,
    /// -1 lets the backend (or we, for ComfyUI) pick one.
    pub seed: i64,
    pub model: String,
    pub sampler: String,
    pub scheduler: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedImage {
    pub data: Vec<u8>,
    pub mime_type: String,
}

/// What the server offers, for the Settings pickers.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendOptions {
    pub models: Vec<String>,
    pub samplers: Vec<String>,
    pub schedulers: Vec<String>,
}

/// Rendering is one long request (A1111) or a long wait (ComfyUI), so the read limit is far longer
/// than the chat client's: a big SDXL picture on a slow card can take several minutes. Grok is
/// reached over HTTPS, so it needs the same certificate roots as the chat client.
pub fn client() -> reqwest::Client {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(900))
        .user_agent(concat!("LocalTavern/", env!("CARGO_PKG_VERSION")));
    crate::providers::http::with_tls_roots(builder)
        .build()
        .expect("reqwest client configuration is static and valid")
}

/// `grok` is the SuperGrok sign-in; only the Grok backend asks it for a token. The on-device lists
/// come from the image models folder, without starting the server.
pub async fn options(
    client: &reqwest::Client,
    settings: &ImageGenSettings,
    grok: &dyn AccessTokenSource,
    on_device: &ondevice::OnDeviceImages,
) -> Result<BackendOptions> {
    match settings.backend {
        ImageBackend::A1111 => a1111::options(client, &settings.a1111.url).await,
        ImageBackend::ComfyUi => comfy::options(client, &settings.comfy.url).await,
        ImageBackend::Grok => grok::options(client, &settings.grok.url, grok).await,
        ImageBackend::OnDevice => Ok(on_device.options()),
    }
}

/// Renders one picture. `Ok(None)` means it was stopped; the server is asked to stop too. For the
/// on-device backend, `settings.on_device.url` must be the running server's address.
pub async fn generate(
    client: &reqwest::Client,
    settings: &ImageGenSettings,
    grok: &dyn AccessTokenSource,
    request: &ImageRequest,
    video: bool,
    cancel: &CancellationToken,
) -> Result<Option<GeneratedImage>> {
    let url = &settings.active().url;
    let work = async {
        match settings.backend {
            // sd-server speaks the A1111 API.
            ImageBackend::A1111 | ImageBackend::OnDevice => {
                if video {
                    return Err(Error::Other("Story videos need the ComfyUI backend.".into()));
                }
                a1111::generate(client, url, request).await
            }
            ImageBackend::ComfyUi => {
                let workflow = if video { &settings.comfy_video_workflow } else { &settings.comfy_workflow };
                comfy::generate(client, url, workflow, request, video).await
            }
            ImageBackend::Grok => {
                if video {
                    return Err(Error::Other("Story videos need the ComfyUI backend.".into()));
                }
                grok::generate(client, url, grok, request).await
            }
        }
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            match settings.backend {
                ImageBackend::A1111 => a1111::interrupt(client, url).await,
                ImageBackend::ComfyUi => comfy::interrupt(client, url).await,
                // Dropping the request is all there is: xAI has no way to cancel one, and sd-server
                // has no interrupt (the caller stops it instead).
                ImageBackend::Grok | ImageBackend::OnDevice => {}
            }
            Ok(None)
        }
        result = work => result.map(Some),
    }
}

/// Best-effort: has the image server give the graphics card back once a picture is done. Only
/// ComfyUI needs telling - the A1111 family frees its working memory after each picture itself.
pub async fn release_vram(client: &reqwest::Client, settings: &ImageGenSettings) {
    if settings.backend == ImageBackend::ComfyUi {
        comfy::free_vram(client, &settings.comfy.url).await;
    }
}

/// A seed every backend accepts (ComfyUI rejects -1) that also survives a round trip through
/// JavaScript numbers.
pub fn random_seed() -> i64 {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    (u64::from_le_bytes(bytes) & ((1 << 48) - 1)) as i64
}

/// Image servers return PNG unless told otherwise, but A1111 follows its "save as" setting, so the
/// type comes from the bytes themselves.
pub fn sniff_mime(data: &[u8]) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'\x1a', b'E', b'\xdf', b'\xa3', ..] => "video/webm",
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => "video/mp4",
        _ => "image/png",
    }
}

/// A transport failure as something the user can act on: most often the server isn't running.
pub(crate) fn unreachable(server: &str, url: &str, e: &reqwest::Error) -> Error {
    if e.is_connect() {
        Error::Other(format!("Couldn't reach {server} at {url} - is it running?"))
    } else if e.is_timeout() {
        Error::Other(format!("{server} at {url} took too long to answer."))
    } else {
        Error::Other(format!("Network error talking to {server} at {url}: {e}"))
    }
}

/// A short excerpt of an error body (servers sometimes answer with a whole HTML page).
pub(crate) fn excerpt(body: &str) -> String {
    let body = body.trim();
    let mut cut: String = body.chars().take(300).collect();
    if cut.len() < body.len() {
        cut.push('…');
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_common_image_types() {
        assert_eq!(sniff_mime(&[0x89, b'P', b'N', b'G', 0x0D]), "image/png");
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(sniff_mime(b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(sniff_mime(b"GIF89a"), "image/gif");
        assert_eq!(sniff_mime(b"\x1aE\xdf\xa3...."), "video/webm");
        assert_eq!(sniff_mime(b"....ftypisom"), "video/mp4");
        assert_eq!(sniff_mime(b""), "image/png");
    }

    #[test]
    fn excerpts_are_cut_on_a_character_boundary() {
        assert_eq!(excerpt("  short  "), "short");
        let long = "é".repeat(400);
        let cut = excerpt(&long);
        assert_eq!(cut.chars().count(), 301);
        assert!(cut.ends_with('…'));
    }
}
