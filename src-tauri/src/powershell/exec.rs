//! Runs a command through PowerShell 7 (`pwsh`, resolved from PATH - never bundled or installed) on
//! behalf of the assistant's PowerShell tool. Two paths: a direct redirected-IO process for the
//! normal case, and - for elevation - a temp script plus output files, since a process started with
//! the `runas` verb can't have its output redirected.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_OUTPUT_CHARS: usize = 4000;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const ELEVATED_POLL_INTERVAL: Duration = Duration::from_millis(300);
/// `CREATE_NO_WINDOW`: no console flashes up for the child.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// What the outer (non-elevated) launcher prints when the user turns the UAC prompt down.
const UAC_DECLINED_MARKER: &str = "LT_UAC_DECLINED";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionResult {
    pub command: String,
    pub elevated: bool,
    pub exit_code: Option<i32>,
    pub std_out: String,
    pub std_err: String,
    pub timed_out: bool,
    pub uac_declined: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecError {
    /// `pwsh` isn't installed / not on PATH. Worded for the model and the user alike.
    #[error("PowerShell 7 (pwsh.exe) wasn't found on PATH. Install it from https://github.com/PowerShell/PowerShell and make sure pwsh is on PATH, then try again.")]
    PwshNotFound,
    #[error("Couldn't start PowerShell: {0}")]
    Spawn(String),
    /// The caller stopped the run.
    #[error("cancelled")]
    Cancelled,
}

/// Something that can run a PowerShell command. The app uses [`Pwsh`]; tests use a fake.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// `on_output` gets each new chunk of stdout/stderr as it arrives (interleaved in arrival
    /// order) - purely for live display, not used to build the returned result.
    async fn run(
        &self,
        command: &str,
        elevate: bool,
        on_output: &(dyn Fn(String) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<ExecutionResult, ExecError>;
}

pub struct Pwsh {
    program: String,
    timeout: Duration,
}

impl Default for Pwsh {
    fn default() -> Self {
        Self { program: "pwsh".into(), timeout: DEFAULT_TIMEOUT }
    }
}

impl Pwsh {
    #[cfg(test)]
    fn custom(program: &str, timeout: Duration) -> Self {
        Self { program: program.into(), timeout }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.stdin(Stdio::null()).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }

    fn spawn(&self, mut command: Command) -> Result<Child, ExecError> {
        command.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ExecError::PwshNotFound,
            _ => ExecError::Spawn(e.to_string()),
        })
    }

    async fn run_direct(
        &self,
        command_text: &str,
        on_output: &(dyn Fn(String) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<ExecutionResult, ExecError> {
        let mut command = self.command();
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", command_text])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = self.spawn(command)?;

        let out = Arc::new(Mutex::new(String::new()));
        let err = Arc::new(Mutex::new(String::new()));
        let sink = Mutex::new(on_output);
        // Both streams are read on this task so `on_output` needn't be 'static.
        let (stdout, stderr) = (child.stdout.take().expect("piped"), child.stderr.take().expect("piped"));
        let (mut out_lines, mut err_lines) = (BufReader::new(stdout).lines(), BufReader::new(stderr).lines());
        let (mut out_open, mut err_open) = (true, true);

        let deadline = tokio::time::sleep(self.timeout);
        tokio::pin!(deadline);
        let mut exit: Option<std::process::ExitStatus> = None;
        let mut timed_out = false;

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    kill_tree(&mut child).await;
                    return Err(ExecError::Cancelled);
                }
                _ = &mut deadline, if !timed_out => {
                    timed_out = true;
                    kill_tree(&mut child).await;
                    // Give the pipes a moment to hand over what was already written, then stop waiting.
                    deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(2));
                    exit = child.try_wait().ok().flatten();
                    if exit.is_none() { break; }
                }
                line = out_lines.next_line(), if out_open => match line {
                    Ok(Some(line)) => push_line(&out, &sink, &line),
                    _ => out_open = false,
                },
                line = err_lines.next_line(), if err_open => match line {
                    Ok(Some(line)) => push_line(&err, &sink, &line),
                    _ => err_open = false,
                },
                status = child.wait(), if exit.is_none() => {
                    exit = status.ok();
                }
            }
            // Finished once the process has exited and both pipes have drained (or, after a
            // timeout, once the short grace period is over).
            if exit.is_some() && !out_open && !err_open {
                break;
            }
            if timed_out && deadline.is_elapsed() {
                break;
            }
        }

        let (std_out, std_err) = (truncate(&out.lock().unwrap()), truncate(&err.lock().unwrap()));
        Ok(ExecutionResult {
            command: command_text.to_string(),
            elevated: false,
            exit_code: if timed_out { None } else { exit.and_then(|s| s.code()) },
            std_out,
            std_err,
            timed_out,
            uac_declined: false,
        })
    }

    async fn run_elevated(
        &self,
        command_text: &str,
        on_output: &(dyn Fn(String) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<ExecutionResult, ExecError> {
        let temp = std::env::temp_dir();
        let id = Uuid::new_v4().simple();
        let script_path = temp.join(format!("ltps_{id}.ps1"));
        let out_path = temp.join(format!("ltps_{id}.out.txt"));
        let err_path = temp.join(format!("ltps_{id}.err.txt"));
        let _cleanup = TempFiles(vec![script_path.clone(), out_path.clone(), err_path.clone()]);

        tokio::fs::write(&script_path, elevated_script(command_text, &out_path, &err_path))
            .await
            .map_err(|e| ExecError::Spawn(format!("couldn't write the temp script: {e}")))?;

        let mut command = self.command();
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &launcher_command(&script_path)])
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = self.spawn(command)?;
        let mut launcher_err = BufReader::new(child.stderr.take().expect("piped")).lines();
        let mut launcher_said = String::new();

        let deadline = tokio::time::sleep(self.timeout);
        tokio::pin!(deadline);
        let mut poll = tokio::time::interval(ELEVATED_POLL_INTERVAL);
        let (mut seen_out, mut seen_err) = (0usize, 0usize);
        let mut timed_out = false;
        let mut err_open = true;

        let status = loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    kill_tree(&mut child).await;
                    return Err(ExecError::Cancelled);
                }
                _ = &mut deadline => {
                    timed_out = true;
                    kill_tree(&mut child).await;
                    break None;
                }
                line = launcher_err.next_line(), if err_open => match line {
                    Ok(Some(line)) => { launcher_said.push_str(&line); launcher_said.push('\n'); }
                    _ => err_open = false,
                },
                _ = poll.tick() => {
                    seen_out = emit_new(&out_path, seen_out, on_output).await;
                    seen_err = emit_new(&err_path, seen_err, on_output).await;
                }
                status = child.wait() => break status.ok(),
            }
        };
        while let Ok(Ok(Some(line))) = tokio::time::timeout(Duration::from_millis(200), launcher_err.next_line()).await {
            launcher_said.push_str(&line);
            launcher_said.push('\n');
        }

        if launcher_said.contains(UAC_DECLINED_MARKER) {
            return Ok(ExecutionResult { command: command_text.into(), elevated: true, uac_declined: true, ..Default::default() });
        }
        let mut std_err = truncate(&read_best_effort(&err_path).await);
        if std_err.is_empty() && !launcher_said.trim().is_empty() && status.and_then(|s| s.code()) != Some(0) {
            // The launcher itself failed (not the command) - say why.
            std_err = truncate(&launcher_said);
        }
        Ok(ExecutionResult {
            command: command_text.to_string(),
            elevated: true,
            exit_code: if timed_out { None } else { status.and_then(|s| s.code()) },
            std_out: truncate(&read_best_effort(&out_path).await),
            std_err,
            timed_out,
            uac_declined: false,
        })
    }
}

#[async_trait]
impl CommandRunner for Pwsh {
    async fn run(
        &self,
        command: &str,
        elevate: bool,
        on_output: &(dyn Fn(String) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<ExecutionResult, ExecError> {
        if elevate {
            self.run_elevated(command, on_output, cancel).await
        } else {
            self.run_direct(command, on_output, cancel).await
        }
    }
}

fn push_line(buffer: &Mutex<String>, on_output: &Mutex<&(dyn Fn(String) + Send + Sync)>, line: &str) {
    let mut buffer = buffer.lock().unwrap();
    buffer.push_str(line);
    buffer.push('\n');
    (on_output.lock().unwrap())(format!("{line}\n"));
}

/// The script the elevated process runs: the command's output goes to two files we can tail.
fn elevated_script(command: &str, out_path: &Path, err_path: &Path) -> String {
    format!("& {{ {command} }} 1> \"{}\" 2> \"{}\"", out_path.display(), err_path.display())
}

/// Runs the script in an elevated pwsh (UAC prompt), waits for it, and passes its exit code on.
/// A declined UAC prompt is reported through [`UAC_DECLINED_MARKER`] on stderr.
fn launcher_command(script_path: &Path) -> String {
    let script = script_path.display().to_string().replace('\'', "''");
    format!(
        "try {{ $p = Start-Process -FilePath pwsh -Verb RunAs -WindowStyle Hidden -Wait -PassThru -ErrorAction Stop \
-ArgumentList '-NoLogo','-NoProfile','-NonInteractive','-File','{script}' }} \
catch {{ if ($_.Exception.Message -match 'canceled by the user' -or $_.Exception.InnerException.NativeErrorCode -eq 1223) \
{{ [Console]::Error.WriteLine('{UAC_DECLINED_MARKER}'); exit 1223 }} \
[Console]::Error.WriteLine($_.Exception.Message); exit 1 }} \
exit $p.ExitCode"
    )
}

/// Reports the text appended to `path` since `seen` chars ago; returns the new length.
async fn emit_new(path: &Path, seen: usize, on_output: &(dyn Fn(String) + Send + Sync)) -> usize {
    let text = read_best_effort(path).await;
    let len = text.chars().count();
    if len > seen {
        on_output(text.chars().skip(seen).collect());
    }
    len.max(seen)
}

async fn read_best_effort(path: &Path) -> String {
    // pwsh redirection writes UTF-8 (no BOM) in PowerShell 7; a stray BOM would only be cosmetic.
    match tokio::fs::read(path).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string(),
        Err(_) => String::new(),
    }
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        text.trim().to_string()
    } else {
        format!("{}\n... (truncated)", text.chars().take(MAX_OUTPUT_CHARS).collect::<String>().trim())
    }
}

/// Kills the process and everything it started.
async fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .await;
    }
    let _ = child.kill().await;
}

/// Deletes the temp files when dropped.
struct TempFiles(Vec<PathBuf>);

impl Drop for TempFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pwsh_available() -> bool {
        std::process::Command::new("pwsh").args(["-NoProfile", "-Command", "exit 0"]).output().is_ok()
    }

    async fn run(command: &str) -> (ExecutionResult, String) {
        let live = Mutex::new(String::new());
        let result = Pwsh::default()
            .run(command, false, &|chunk| live.lock().unwrap().push_str(&chunk), &CancellationToken::new())
            .await
            .unwrap();
        let live = live.into_inner().unwrap();
        (result, live)
    }

    #[tokio::test]
    async fn output_exit_code_and_stderr_are_captured_and_streamed() {
        if !pwsh_available() {
            return eprintln!("skipped: pwsh isn't installed");
        }
        let (result, live) = run("Write-Output 'hello'; [Console]::Error.WriteLine('oops'); exit 3").await;
        assert_eq!(result.std_out, "hello");
        assert_eq!(result.std_err, "oops");
        assert_eq!(result.exit_code, Some(3));
        assert!(!result.timed_out && !result.elevated && !result.uac_declined);
        assert!(live.contains("hello\n") && live.contains("oops\n"), "{live:?}");
    }

    #[tokio::test]
    async fn a_command_error_surfaces_and_multiline_scripts_work() {
        if !pwsh_available() {
            return;
        }
        let (result, _) = run("$a = 20\n$b = 22\nWrite-Output ($a + $b)").await;
        assert_eq!((result.std_out.as_str(), result.exit_code), ("42", Some(0)));

        let (failed, _) = run("Get-Item C:\\this\\does\\not\\exist -ErrorAction Stop").await;
        assert!(failed.std_err.contains("does not exist") || failed.std_err.contains("Cannot find"), "{}", failed.std_err);
        assert_eq!(failed.exit_code, Some(1));
    }

    #[tokio::test]
    async fn long_output_is_truncated() {
        if !pwsh_available() {
            return;
        }
        let (result, _) = run("'x' * 9000").await;
        assert!(result.std_out.ends_with("... (truncated)"));
        assert!(result.std_out.chars().count() < 4100);
    }

    #[tokio::test]
    async fn a_hung_command_is_killed_after_the_timeout_keeping_what_it_printed() {
        if !pwsh_available() {
            return;
        }
        let started = std::time::Instant::now();
        let result = Pwsh::custom("pwsh", Duration::from_millis(1500))
            .run("Write-Output 'before'; Start-Sleep -Seconds 60", false, &|_| {}, &CancellationToken::new())
            .await
            .unwrap();
        assert!(result.timed_out);
        assert_eq!(result.exit_code, None);
        assert_eq!(result.std_out, "before");
        assert!(started.elapsed() < Duration::from_secs(20), "it did not wait for the sleep");
    }

    #[tokio::test]
    async fn cancelling_kills_the_command() {
        if !pwsh_available() {
            return;
        }
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            trigger.cancel();
        });
        let started = std::time::Instant::now();
        let result = Pwsh::default().run("Start-Sleep -Seconds 60", false, &|_| {}, &cancel).await;
        assert_eq!(result, Err(ExecError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[tokio::test]
    async fn a_missing_pwsh_is_reported_as_such() {
        let result = Pwsh::custom("definitely-not-a-real-pwsh-binary", DEFAULT_TIMEOUT)
            .run("Get-Date", false, &|_| {}, &CancellationToken::new())
            .await;
        assert_eq!(result, Err(ExecError::PwshNotFound));
        let elevated = Pwsh::custom("definitely-not-a-real-pwsh-binary", DEFAULT_TIMEOUT)
            .run("Get-Date", true, &|_| {}, &CancellationToken::new())
            .await;
        assert_eq!(elevated, Err(ExecError::PwshNotFound));
        assert!(ExecError::PwshNotFound.to_string().contains("github.com/PowerShell/PowerShell"));
    }

    #[test]
    fn the_elevated_script_redirects_output_and_the_launcher_quotes_its_path() {
        let script = elevated_script("Get-Date", Path::new("C:\\t\\o.txt"), Path::new("C:\\t\\e.txt"));
        assert_eq!(script, "& { Get-Date } 1> \"C:\\t\\o.txt\" 2> \"C:\\t\\e.txt\"");
        let launcher = launcher_command(Path::new("C:\\it's\\x.ps1"));
        assert!(launcher.contains("'C:\\it''s\\x.ps1'"), "{launcher}");
        assert!(launcher.contains("-Verb RunAs") && launcher.contains(UAC_DECLINED_MARKER) && launcher.ends_with("exit $p.ExitCode"));
    }

    /// The launcher's own syntax must parse, since elevation itself can't be exercised unattended.
    #[tokio::test]
    async fn the_launcher_command_is_valid_powershell() {
        if !pwsh_available() {
            return;
        }
        let launcher = launcher_command(Path::new("C:\\x.ps1"));
        let check = format!("$null = [scriptblock]::Create({}); 'parsed'", ps_string(&launcher));
        let (result, _) = run(&check).await;
        assert_eq!(result.std_out, "parsed", "{}", result.std_err);
    }

    fn ps_string(text: &str) -> String {
        format!("'{}'", text.replace('\'', "''"))
    }

    #[test]
    fn truncation_keeps_short_text_and_trims_it() {
        assert_eq!(truncate("  hi \n"), "hi");
        assert!(truncate(&"é".repeat(5000)).ends_with("... (truncated)"));
    }
}
