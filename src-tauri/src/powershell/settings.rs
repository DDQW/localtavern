//! Persisted options for the assistant's PowerShell tool - off by default, since it's an opt-in,
//! high-trust capability. Stored under the same key and JSON shape as the C# app.

use regex::{Regex, RegexBuilder};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::repo::app_settings;

const KEY: &str = "powershell.toolOptions";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PowerShellToolOptions {
    pub enabled: bool,
    /// Runs non-elevated commands without asking. Elevation always asks.
    pub auto_mode: bool,
    /// Commands matching one of these (`*` wildcards, case-insensitive) run without asking.
    pub allowlist_patterns: Vec<String>,
}

impl PowerShellToolOptions {
    pub fn allows(&self, command: &str) -> bool {
        self.allowlist_patterns.iter().any(|p| matches_pattern(command, p))
    }
}

/// Simple glob match (`*` only) against a stored allowlist pattern, case-insensitive.
pub fn matches_pattern(command: &str, pattern: &str) -> bool {
    let escaped: Vec<String> = pattern.split('*').map(regex::escape).collect();
    let regex: Option<Regex> = RegexBuilder::new(&format!("^(?s){}$", escaped.join(".*"))).case_insensitive(true).build().ok();
    regex.is_some_and(|r| r.is_match(command))
}

pub fn load(conn: &Connection) -> Result<PowerShellToolOptions> {
    // Unreadable stored settings fall back to the safe default (off).
    Ok(app_settings::get(conn, KEY)?
        .filter(|j| !j.is_empty())
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default())
}

pub fn save(conn: &Connection, options: &PowerShellToolOptions) -> Result<()> {
    let mut clean = options.clone();
    clean.allowlist_patterns.retain(|p| !p.trim().is_empty());
    app_settings::set(conn, KEY, &serde_json::to_string(&clean)?)
}

/// Adds one pattern to the allowlist (the approval dialog's "Always allow"); a no-op if present.
pub fn add_allowlist_pattern(conn: &Connection, pattern: &str) -> Result<()> {
    let pattern = pattern.trim();
    let mut options = load(conn)?;
    if pattern.is_empty() || options.allowlist_patterns.iter().any(|p| p.eq_ignore_ascii_case(pattern)) {
        return Ok(());
    }
    options.allowlist_patterns.push(pattern.to_string());
    save(conn, &options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_are_case_insensitive_globs_where_only_star_is_special() {
        assert!(matches_pattern("Get-Process", "get-process"));
        assert!(matches_pattern("Get-Process -Name x", "Get-Process*"));
        assert!(matches_pattern("Get-ChildItem C:\\tmp", "*C:\\tmp"));
        assert!(matches_pattern("a b c", "a*c"));
        assert!(!matches_pattern("Get-Process; rm x", "Get-Process"));
        assert!(!matches_pattern("xGet-Process", "Get-Process*"));
        // Regex characters in a pattern are literal.
        assert!(matches_pattern("echo (a+b)", "echo (a+b)"));
        assert!(!matches_pattern("echo aab", "echo (a+b)"));
        assert!(matches_pattern("line1\nline2", "line1*"), "multi-line commands match too");
    }

    #[test]
    fn options_round_trip_in_the_c_sharp_shape_and_default_to_off() {
        let conn = crate::db::open_in_memory().unwrap();
        assert_eq!(load(&conn).unwrap(), PowerShellToolOptions::default());
        assert!(!load(&conn).unwrap().enabled);

        save(&conn, &PowerShellToolOptions { enabled: true, auto_mode: false, allowlist_patterns: vec!["ls*".into(), "  ".into()] })
            .unwrap();
        assert_eq!(app_settings::get(&conn, KEY).unwrap().unwrap(), r#"{"Enabled":true,"AutoMode":false,"AllowlistPatterns":["ls*"]}"#);

        app_settings::set(&conn, KEY, r#"{"Enabled":true,"AutoMode":true,"AllowlistPatterns":["a"]}"#).unwrap();
        let loaded = load(&conn).unwrap();
        assert!(loaded.enabled && loaded.auto_mode);
        assert!(loaded.allows("A"));
    }

    #[test]
    fn corrupt_settings_mean_off() {
        let conn = crate::db::open_in_memory().unwrap();
        app_settings::set(&conn, KEY, "{nope").unwrap();
        assert_eq!(load(&conn).unwrap(), PowerShellToolOptions::default());
    }

    #[test]
    fn allowlist_additions_skip_blanks_and_duplicates() {
        let conn = crate::db::open_in_memory().unwrap();
        add_allowlist_pattern(&conn, " Get-Date ").unwrap();
        add_allowlist_pattern(&conn, "get-date").unwrap();
        add_allowlist_pattern(&conn, "   ").unwrap();
        assert_eq!(load(&conn).unwrap().allowlist_patterns, ["Get-Date"]);
    }
}
