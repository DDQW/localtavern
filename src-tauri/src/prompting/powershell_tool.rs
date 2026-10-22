//! The prompt-based protocol that lets an Assistant-mode conversation offer the model a PowerShell
//! tool without any provider needing native function-calling support: the model is instructed to
//! emit a specific tag, which is then parsed back out of its own text. Unlike the emotion tag it has
//! to be recognized mid-stream (so the raw tag never flashes in the chat bubble) - the stream filter
//! uses [`OPEN_TAG`]/[`CLOSE_TAG`] to find the boundaries and [`try_parse`] to interpret the inside.

use std::sync::LazyLock;

use regex::Regex;

use super::instructions::{self, Instructions};

/// The literal text that opens a tool call - may be followed by the elevate attribute and then `>`.
pub const OPEN_TAG: &str = "<powershell-run";
pub const CLOSE_TAG: &str = "</powershell-run>";

static TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)^<powershell-run(?:\s+elevate\s*=\s*"(?P<elevate>true|false)")?\s*>(?P<command>.*)</powershell-run>\n?$"#)
        .unwrap()
});

/// A parsed `<powershell-run>` request pulled out of the model's own reply text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerShellToolCallRequest {
    pub command: String,
    pub elevate: bool,
}

pub fn build_instruction(ins: &Instructions) -> String {
    ins.render(instructions::POWERSHELL_TOOL, &[])
}

/// Parses one complete `<powershell-run ...>...</powershell-run>` tag. `None` if `tag_text` isn't a
/// well-formed tag of this shape or the command is empty.
pub fn try_parse(tag_text: &str) -> Option<PowerShellToolCallRequest> {
    let caps = TAG.captures(tag_text)?;
    let command = caps["command"].trim().to_string();
    if command.is_empty() {
        return None;
    }
    let elevate = caps.name("elevate").is_some_and(|m| m.as_str() == "true");
    Some(PowerShellToolCallRequest { command, elevate })
}

/// A finished (or refused) command, as reported back to the model.
#[derive(Debug, Clone, Default)]
pub struct ToolResult<'a> {
    pub command: &'a str,
    pub elevated: bool,
    pub exit_code: Option<i32>,
    pub std_out: &'a str,
    pub std_err: &'a str,
    pub timed_out: bool,
    pub declined: bool,
}

/// Formats a tool result so it's unambiguous to the model that this is real command output, not
/// something the user typed.
pub fn format_result_for_model(r: &ToolResult) -> String {
    if r.declined {
        return format!(
            "[PowerShell tool result]\nCommand: {}\nThe user declined to run this command.",
            r.command
        );
    }

    let mut lines = vec![
        "[PowerShell tool result]".to_string(),
        format!("Command: {}", r.command),
        format!("Elevated: {}", if r.elevated { "yes" } else { "no" }),
    ];

    if r.timed_out {
        lines.push("The command timed out and was terminated.".into());
    } else if let Some(code) = r.exit_code {
        lines.push(format!("Exit code: {code}"));
    }
    if !r.std_out.is_empty() {
        lines.push("Output:".into());
        lines.push(r.std_out.into());
    }
    if !r.std_err.is_empty() {
        lines.push("Error output:".into());
        lines.push(r.std_err.into());
    }
    if r.std_out.is_empty() && r.std_err.is_empty() && !r.timed_out {
        lines.push("(No output.)".into());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_plain_command() {
        let req = try_parse("<powershell-run>Get-Process</powershell-run>").unwrap();
        assert_eq!(req, PowerShellToolCallRequest { command: "Get-Process".into(), elevate: false });
    }

    #[test]
    fn reads_the_elevate_attribute() {
        let req = try_parse("<powershell-run elevate=\"true\">Restart-Service Spooler</powershell-run>").unwrap();
        assert_eq!(req.command, "Restart-Service Spooler");
        assert!(req.elevate);
        let no = try_parse("<powershell-run elevate=\"false\">x</powershell-run>").unwrap();
        assert!(!no.elevate);
    }

    #[test]
    fn trims_surrounding_whitespace_in_the_command() {
        let req = try_parse("<powershell-run>\n  Get-ChildItem  \n</powershell-run>").unwrap();
        assert_eq!(req.command, "Get-ChildItem");
    }

    #[test]
    fn keeps_multiline_commands() {
        let req = try_parse("<powershell-run>$a = 1\n$a + 1</powershell-run>").unwrap();
        assert_eq!(req.command, "$a = 1\n$a + 1");
    }

    #[test]
    fn rejects_empty_commands() {
        assert!(try_parse("<powershell-run></powershell-run>").is_none());
        assert!(try_parse("<powershell-run>   </powershell-run>").is_none());
    }

    #[test]
    fn does_not_false_positive_on_a_code_fence() {
        assert!(try_parse("```powershell\nGet-Process\n```").is_none());
    }

    #[test]
    fn instruction_mentions_the_tag_and_elevation() {
        let instruction = build_instruction(instructions::defaults());
        assert!(instruction.contains("<powershell-run>"));
        assert!(instruction.contains("elevate"));
    }

    #[test]
    fn result_reports_a_decline() {
        let text = format_result_for_model(&ToolResult { command: "Get-Process", declined: true, ..Default::default() });
        assert!(text.contains("declined"));
        assert!(text.contains("Get-Process"));
    }

    #[test]
    fn result_includes_output_and_exit_code() {
        let text = format_result_for_model(&ToolResult {
            command: "Get-Date",
            exit_code: Some(0),
            std_out: "Monday",
            ..Default::default()
        });
        assert!(text.contains("Exit code: 0"));
        assert!(text.contains("Output:\nMonday"));
        assert!(text.contains("Elevated: no"));
    }

    #[test]
    fn result_reports_timeouts_stderr_and_empty_output() {
        let timed_out = format_result_for_model(&ToolResult { command: "x", timed_out: true, ..Default::default() });
        assert!(timed_out.contains("timed out"));
        assert!(!timed_out.contains("(No output.)"));

        let err = format_result_for_model(&ToolResult { command: "x", exit_code: Some(1), std_err: "boom", ..Default::default() });
        assert!(err.contains("Error output:\nboom"));

        let empty = format_result_for_model(&ToolResult { command: "x", exit_code: Some(0), ..Default::default() });
        assert!(empty.ends_with("(No output.)"));
    }
}
