//! The llama-server process: one model at a time, on a free localhost port. It is started the first
//! time a reply needs it and killed when it has been idle a while or another model is wanted, so the
//! gigabytes it holds go back to the phone.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::providers::error::ProviderError;

/// How many of the server's last log lines are kept, to explain a failed start.
const LOG_LINES: usize = 40;

/// What to run. Two specs that are equal can share one running server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    pub binary: PathBuf,
    /// Where the server's own libraries (ggml backends, libc++) are.
    pub lib_dir: PathBuf,
    pub model: PathBuf,
    pub projector: Option<PathBuf>,
    pub context_size: i32,
    pub threads: i32,
}

impl ServerSpec {
    pub fn args(&self, port: u16) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--model".into(),
            self.model.to_string_lossy().into_owned(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
            "--ctx-size".into(),
            self.context_size.to_string(),
            // One request at a time gets the whole context and all the cores. A second slot for the
            // background requests after a reply didn't make the next reply come sooner: a phone's
            // CPU is the bottleneck, so running them side by side only splits it.
            "--parallel".into(),
            "1".into(),
            "--threads".into(),
            self.threads.to_string(),
            "--threads-batch".into(),
            self.threads.to_string(),
            // The model's own chat template, so any GGUF formats its turns correctly.
            "--jinja".into(),
            "--flash-attn".into(),
            "on".into(),
            // Half the memory for the context, at no noticeable cost in quality.
            "--cache-type-k".into(),
            "q8_0".into(),
            "--cache-type-v".into(),
            "q8_0".into(),
            // Reuse matching stretches of the previous prompt even after an earlier part changed.
            "--cache-reuse".into(),
            "256".into(),
            // Earlier prompts are kept in RAM so a background request between two replies doesn't
            // cost the chat its cached prompt; the default allowance (8 GB) is far too much here.
            "--cache-ram".into(),
            "1024".into(),
            "--no-webui".into(),
        ];
        if let Some(projector) = &self.projector {
            args.extend(["--mmproj".into(), projector.to_string_lossy().into_owned()]);
        }
        args
    }
}

pub struct Server {
    pub spec: ServerSpec,
    pub base_url: String,
    child: Child,
}

impl Server {
    /// Starts the server and waits until it has loaded the model and answers.
    pub async fn start(spec: ServerSpec, client: &reqwest::Client, ready_timeout: Duration) -> Result<Server, ProviderError> {
        let port = free_port()?;
        let mut child = Command::new(&spec.binary)
            .args(spec.args(port))
            .env("LD_LIBRARY_PATH", &spec.lib_dir)
            .current_dir(&spec.lib_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ProviderError::network(format!("Couldn't start the on-device model server: {e}")))?;

        let log = Arc::new(Mutex::new(VecDeque::with_capacity(LOG_LINES)));
        if let Some(stderr) = child.stderr.take() {
            let log = log.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut log = log.lock().unwrap();
                    if log.len() == LOG_LINES {
                        log.pop_front();
                    }
                    log.push_back(line);
                }
            });
        }

        let base_url = format!("http://127.0.0.1:{port}");
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(ProviderError::network(format!(
                    "The on-device model server stopped while loading ({status}).{}",
                    explain(&log)
                )));
            }
            let health = client.get(format!("{base_url}/health")).timeout(Duration::from_secs(2)).send().await;
            if health.is_ok_and(|r| r.status().is_success()) {
                return Ok(Server { spec, base_url, child });
            }
            if started.elapsed() > ready_timeout {
                let _ = child.kill().await;
                return Err(ProviderError::network(format!(
                    "The on-device model didn't finish loading within {} seconds.{}",
                    ready_timeout.as_secs(),
                    explain(&log)
                )));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Still running - it may have been killed for memory since the last reply.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
    }
}

/// The last lines of the server's log, which usually say why it failed (e.g. out of memory).
fn explain(log: &Mutex<VecDeque<String>>) -> String {
    let log = log.lock().unwrap();
    let tail: Vec<&str> = log.iter().rev().take(6).rev().map(String::as_str).collect();
    if tail.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", tail.join("\n"))
    }
}

fn free_port() -> Result<u16, ProviderError> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| ProviderError::network(format!("No free local port for the model server: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(projector: Option<&str>) -> ServerSpec {
        ServerSpec {
            binary: "libllama-server.so".into(),
            lib_dir: "lib".into(),
            model: "models/Luchador.gguf".into(),
            projector: projector.map(PathBuf::from),
            context_size: 8192,
            threads: 4,
        }
    }

    #[test]
    fn args_load_the_model_on_localhost_and_add_the_projector_only_when_set() {
        let args = spec(None).args(4321);
        let joined = args.join(" ");
        assert!(joined.contains("--model models/Luchador.gguf --host 127.0.0.1 --port 4321"), "{joined}");
        assert!(joined.contains("--ctx-size 8192 --parallel 1 --threads 4 --threads-batch 4"), "{joined}");
        assert!(!joined.contains("--mmproj"), "{joined}");

        let with = spec(Some("models/mmproj.gguf")).args(4321).join(" ");
        assert!(with.ends_with("--mmproj models/mmproj.gguf"), "{with}");
    }

    #[tokio::test]
    async fn a_binary_that_isnt_there_is_a_clear_error() {
        let err = Server::start(spec(None), &reqwest::Client::new(), Duration::from_secs(1)).await.err().unwrap();
        assert!(err.message.starts_with("Couldn't start the on-device model server"), "{}", err.message);
    }
}
