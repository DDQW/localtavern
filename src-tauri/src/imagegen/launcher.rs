//! Starts ComfyUI for the user: the launcher from Settings (ComfyUI's own `run_*.bat`, or any
//! script or program) is run hidden when LocalTavern starts, and a picture asked for while it is
//! still starting waits for it. Only a server we started is stopped when the app closes; one that
//! was already running is left alone.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::{ImageBackend, ImageGenSettings};
use crate::error::{Error, Result};

/// A cold ComfyUI start (Python, torch, custom nodes) takes 20-60 s; leave room for a slow disk.
const STARTUP_LIMIT: Duration = Duration::from_secs(180);
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// `CREATE_NO_WINDOW`: the launcher and the Python it starts share a console nobody sees.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct ComfyLauncher {
    child: Mutex<Option<Child>>,
    /// The server's console output, rewritten on each start.
    log_path: PathBuf,
}

impl ComfyLauncher {
    pub fn new(log_path: PathBuf) -> Self {
        Self { child: Mutex::new(None), log_path }
    }

    /// Starts ComfyUI unless it already answers or our earlier start is still running. Doesn't wait.
    pub async fn start(&self, client: &reqwest::Client, settings: &ImageGenSettings) -> Result<()> {
        let Some(launcher) = wanted(settings) else {
            return Ok(());
        };
        if !responds(client, &settings.comfy.url).await {
            self.spawn(launcher)?;
        }
        Ok(())
    }

    /// Makes sure ComfyUI answers before a picture is sent to it, starting it if needed. Returns
    /// early (Ok) when `cancel` fires; the render that follows sees the cancel too.
    pub async fn ensure_ready(&self, client: &reqwest::Client, settings: &ImageGenSettings, cancel: &CancellationToken) -> Result<()> {
        let Some(launcher) = wanted(settings) else {
            return Ok(());
        };
        let url = &settings.comfy.url;
        if responds(client, url).await {
            return Ok(());
        }
        self.spawn(launcher)?;
        let deadline = Instant::now() + STARTUP_LIMIT;
        loop {
            if responds(client, url).await {
                return Ok(());
            }
            if let Some(code) = self.exit_code() {
                return Err(Error::Other(format!(
                    "ComfyUI stopped while starting ({code}). Its output is in {}:\n{}",
                    self.log_path.display(),
                    log_tail(&self.log_path)
                )));
            }
            if Instant::now() >= deadline {
                return Err(Error::Other(format!(
                    "ComfyUI didn't answer at {url} within {} s of starting. Its output is in {}.",
                    STARTUP_LIMIT.as_secs(),
                    self.log_path.display()
                )));
            }
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
            }
        }
    }

    /// Stops the server we started, with everything it started (the launcher is usually a batch
    /// file, and the server is its Python child).
    pub fn stop(&self) {
        let Some(mut child) = self.child.lock().unwrap().take() else {
            return;
        };
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let killed = Command::new("taskkill")
                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status();
            if killed.is_ok_and(|s| s.success()) {
                let _ = child.wait();
                return;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Runs the launcher unless our earlier start is still running.
    fn spawn(&self, launcher: &str) -> Result<()> {
        let mut slot = self.child.lock().unwrap();
        if let Some(child) = slot.as_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                return Ok(());
            }
        }
        let path = Path::new(launcher);
        if !path.is_file() {
            return Err(Error::Other(format!("The ComfyUI launcher {launcher} doesn't exist (Settings → Pictures).")));
        }
        let log = File::create(&self.log_path)
            .map_err(|e| Error::Other(format!("Couldn't create {}: {e}", self.log_path.display())))?;
        let log_err = log.try_clone().map_err(|e| Error::Other(e.to_string()))?;
        let mut command = Command::new(path);
        command
            .current_dir(path.parent().unwrap_or(Path::new(".")))
            // With no input a batch file's closing `pause` returns at once instead of waiting.
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log_err)
            // Python buffers redirected output; the log should show a failing start as it happens.
            .env("PYTHONUNBUFFERED", "1");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let child = command.spawn().map_err(|e| Error::Other(format!("Couldn't start ComfyUI with {launcher}: {e}")))?;
        eprintln!("Started ComfyUI with {launcher} (pid {})", child.id());
        *slot = Some(child);
        Ok(())
    }

    /// How our start ended, once it has.
    fn exit_code(&self) -> Option<String> {
        let mut slot = self.child.lock().unwrap();
        let status = slot.as_mut()?.try_wait().ok()??;
        Some(match status.code() {
            Some(code) => format!("exit code {code}"),
            None => status.to_string(),
        })
    }
}

/// The launcher to run, when ComfyUI is the picture server and one is set.
fn wanted(settings: &ImageGenSettings) -> Option<&str> {
    let launcher = settings.comfy_launcher.trim();
    (settings.backend == ImageBackend::ComfyUi && !launcher.is_empty()).then_some(launcher)
}

async fn responds(client: &reqwest::Client, url: &str) -> bool {
    let answer = client.get(format!("{url}/system_stats")).timeout(Duration::from_secs(3)).send().await;
    answer.is_ok_and(|r| r.status().is_success())
}

/// The last few lines of the server's output: usually the traceback that ended it.
fn log_tail(path: &Path) -> String {
    let text = std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(8)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn comfy(url: &str, launcher: &str) -> ImageGenSettings {
        let mut settings = ImageGenSettings { backend: ImageBackend::ComfyUi, comfy_launcher: launcher.into(), ..Default::default() };
        settings.comfy.url = url.into();
        settings
    }

    #[test]
    fn only_runs_for_comfyui_with_a_launcher() {
        assert_eq!(wanted(&comfy("http://x", "  C:\\c\\run.bat ")), Some("C:\\c\\run.bat"));
        assert_eq!(wanted(&comfy("http://x", "  ")), None);
        let a1111 = ImageGenSettings { comfy_launcher: "C:\\c\\run.bat".into(), ..Default::default() };
        assert_eq!(wanted(&a1111), None);
    }

    #[tokio::test]
    async fn a_running_server_is_used_as_it_is() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/system_stats")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let launcher = ComfyLauncher::new(dir.path().join("comfyui.log"));
        // The launcher doesn't exist, so trying to run it would fail.
        let settings = comfy(&server.uri(), "Z:\\nowhere\\run.bat");
        launcher.ensure_ready(&reqwest::Client::new(), &settings, &CancellationToken::new()).await.unwrap();
        launcher.start(&reqwest::Client::new(), &settings).await.unwrap();
        assert!(launcher.child.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_missing_launcher_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = ComfyLauncher::new(dir.path().join("comfyui.log"));
        let settings = comfy("http://127.0.0.1:9", &dir.path().join("run.bat").to_string_lossy());
        let err = launcher.ensure_ready(&reqwest::Client::new(), &settings, &CancellationToken::new()).await.unwrap_err();
        assert!(err.to_string().contains("doesn't exist"), "{err}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_start_that_fails_shows_the_end_of_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("run.bat");
        std::fs::write(&script, "@echo off\r\necho Booting\r\necho ModuleNotFoundError: torch 1>&2\r\nexit /b 3\r\n").unwrap();
        let launcher = ComfyLauncher::new(dir.path().join("comfyui.log"));
        let settings = comfy("http://127.0.0.1:9", &script.to_string_lossy());
        let err = launcher.ensure_ready(&reqwest::Client::new(), &settings, &CancellationToken::new()).await.unwrap_err();
        let text = err.to_string();
        assert!(text.contains("exit code 3") && text.contains("Booting") && text.contains("ModuleNotFoundError: torch"), "{text}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn stop_ends_the_launcher_and_what_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("run.bat");
        // Like ComfyUI's own launchers: a batch file that waits on a program it started.
        std::fs::write(&script, "@echo off\r\ncmd /c ping -n 60 127.0.0.1 >nul\r\n").unwrap();
        let launcher = ComfyLauncher::new(dir.path().join("comfyui.log"));
        launcher.spawn(&script.to_string_lossy()).unwrap();
        let pid = launcher.child.lock().unwrap().as_ref().unwrap().id();
        tokio::time::sleep(Duration::from_millis(500)).await;
        launcher.stop();
        assert!(launcher.child.lock().unwrap().is_none());
        let tasks = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().unwrap();
        assert!(!String::from_utf8_lossy(&tasks.stdout).contains(&pid.to_string()));
    }
}
