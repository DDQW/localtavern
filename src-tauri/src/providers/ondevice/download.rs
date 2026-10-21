//! Downloads a GGUF into the models folder. It is written to `<name>.part` and renamed when complete,
//! so the model list never shows half a file; a cancelled or broken download keeps its `.part` and
//! picks up where it stopped next time, which matters for files of several gigabytes on a phone.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::StreamExt;
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::providers::error::ProviderError;

/// Progress is reported at most this often.
const REPORT_EVERY: Duration = Duration::from_millis(500);

/// The file name a download URL saves as: its last path segment, which must be a `.gguf`.
pub fn file_name(url: &str) -> Result<String, ProviderError> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| ProviderError::invalid_request("That isn't a valid link."))?;
    let last = parsed.path_segments().and_then(|mut s| s.next_back()).unwrap_or_default();
    let name = percent_encoding::percent_decode_str(last).decode_utf8_lossy().into_owned();
    let valid = name.to_ascii_lowercase().ends_with(".gguf") && !name.contains(['/', '\\']) && !name.starts_with('.');
    if !valid {
        return Err(ProviderError::invalid_request(
            "The link has to point straight at a .gguf file (on Hugging Face: the file's download link, with /resolve/ in it).",
        ));
    }
    Ok(name)
}

/// Downloads `url` into `dir`, calling `progress(received, total)` as it goes. Returns the finished file.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    dir: &Path,
    cancel: &CancellationToken,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, ProviderError> {
    let name = file_name(url)?;
    let target = dir.join(&name);
    let part = dir.join(format!("{name}.part"));
    let io = |e: std::io::Error| ProviderError::unknown(format!("Couldn't save {name}: {e}"));
    tokio::fs::create_dir_all(dir).await.map_err(io)?;

    let mut have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    let mut request = client.get(url.trim());
    if have > 0 {
        request = request.header(RANGE, format!("bytes={have}-"));
    }
    let response = request.send().await.map_err(|e| ProviderError::network(format!("Download failed: {e}")))?;
    let status = response.status();
    let resumed = status == StatusCode::PARTIAL_CONTENT;
    if status == StatusCode::RANGE_NOT_SATISFIABLE && have > 0 {
        // The .part already holds the whole file.
        tokio::fs::rename(&part, &target).await.map_err(io)?;
        return Ok(target);
    }
    if !status.is_success() {
        return Err(ProviderError::network(format!("Download failed: {}", super::super::http::describe_status(status))));
    }
    if !resumed {
        have = 0;
    }
    let total = if resumed {
        response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|t| t.parse().ok())
    } else {
        response.content_length()
    };

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .await
        .map_err(io)?;
    let mut body = response.bytes_stream();
    let mut reported = Instant::now() - REPORT_EVERY;
    loop {
        let chunk = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                file.flush().await.map_err(io)?;
                return Err(ProviderError::network("Download stopped. Start it again to continue where it left off."));
            }
            chunk = body.next() => chunk,
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|e| ProviderError::network(format!("Download broke off: {e}. Start it again to continue.")))?;
        file.write_all(&chunk).await.map_err(io)?;
        have += chunk.len() as u64;
        if reported.elapsed() >= REPORT_EVERY {
            reported = Instant::now();
            progress(have, total);
        }
    }
    file.flush().await.map_err(io)?;
    drop(file);
    if total.is_some_and(|t| have < t) {
        return Err(ProviderError::network("The download ended early. Start it again to continue."));
    }
    progress(have, total);
    tokio::fs::rename(&part, &target).await.map_err(io)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn names_the_file_after_the_link_and_wants_a_gguf() {
        let url = "https://huggingface.co/bartowski/X-GGUF/resolve/main/X-Q4_K_M.gguf?download=true";
        assert_eq!(file_name(url).unwrap(), "X-Q4_K_M.gguf");
        assert_eq!(file_name("https://host/a/My%20Model.gguf").unwrap(), "My Model.gguf");
        assert!(file_name("https://huggingface.co/bartowski/X-GGUF").is_err());
        assert!(file_name("https://host/%2E%2E%2Fevil.gguf").is_err());
        assert!(file_name("not a link").is_err());
    }

    #[tokio::test]
    async fn downloads_to_a_part_file_then_renames_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/m.gguf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"GGUFdata".to_vec()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let mut last = (0, None);

        let file = download(&reqwest::Client::new(), &format!("{}/m.gguf", server.uri()), dir.path(), &CancellationToken::new(), |h, t| last = (h, t))
            .await
            .unwrap();

        assert_eq!(std::fs::read(&file).unwrap(), b"GGUFdata");
        assert!(!dir.path().join("m.gguf.part").exists());
        assert_eq!(last, (8, Some(8)));
    }

    #[tokio::test]
    async fn resumes_a_part_file_with_a_range_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("range", "bytes=4-"))
            .respond_with(
                ResponseTemplate::new(206).insert_header("content-range", "bytes 4-7/8").set_body_bytes(b"data".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("m.gguf.part"), b"GGUF").unwrap();

        let file = download(&reqwest::Client::new(), &format!("{}/m.gguf", server.uri()), dir.path(), &CancellationToken::new(), |_, _| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(file).unwrap(), b"GGUFdata");
    }

    #[tokio::test]
    async fn a_server_that_ignores_the_range_starts_over() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"GGUFdata".to_vec()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("m.gguf.part"), b"junkjunkjunk").unwrap();

        let file = download(&reqwest::Client::new(), &format!("{}/m.gguf", server.uri()), dir.path(), &CancellationToken::new(), |_, _| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(file).unwrap(), b"GGUFdata");
    }

    #[tokio::test]
    async fn a_cancelled_download_keeps_its_part_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 64]).set_delay(Duration::from_millis(200)))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = download(&reqwest::Client::new(), &format!("{}/m.gguf", server.uri()), dir.path(), &cancel, |_, _| {})
            .await
            .unwrap_err();
        assert!(err.message.contains("Download stopped"), "{}", err.message);
        assert!(dir.path().join("m.gguf.part").exists());
    }
}
