//! External bump-decision sources.
//!
//! Conventional-commit analysis is the default belaf bump-inference path, but
//! it doesn't see *every* signal that matters in a real release. A GraphQL
//! schema diff knows whether a field rename is breaking; a Maven dependency
//! audit knows whether a transitive bump warrants a major. Bump-sources let
//! the user feed those decisions back into belaf without forking the bump
//! logic.
//!
//! # Wire format (`version: 1`)
//!
//! ```json
//! {
//!   "version": 1,
//!   "decisions": [
//!     {
//!       "project": "graphql-schema",
//!       "bump": "minor",
//!       "reason": "added optional field User.preferredName",
//!       "source": "graphql-inspector"
//!     }
//!   ]
//! }
//! ```
//!
//! `project` is the user-facing name. `bump` is one of `major | minor |
//! patch`. `reason` and `source` are diagnostic-only — belaf logs them but
//! doesn't act on them.
//!
//! # Inputs
//!
//! Three ways to feed decisions in:
//!
//! - `--bump-source <FILE>` — read JSON from a file.
//! - `--bump-source -` — read JSON from stdin (CI-only; rejected in TUI mode).
//! - `--bump-source-cmd <CMD>` — run a shell command, capture stdout as JSON.
//! - `[[bump_source]]` in `belaf/config.toml` — declared subprocesses run by
//!   default. `cmd` field, `timeout_sec` field (default 60s), optional
//!   `project` or `group` filter recorded for diagnostics.
//!
//! # Subprocess discipline
//!
//! - 60s default timeout, configurable per `[[bump_source]]`.
//! - stderr streamed to tracing INFO line-by-line.
//! - non-zero exit or timeout = hard error, no silent fallback.
//!
//! # Precedence
//!
//! Higher beats lower (handled in `cmd::prepare`):
//!
//! 1. `--project name:bump` (per-project CLI override)
//! 2. `--bump-source` / `--bump-source-cmd` (CLI explicit)
//! 3. `[[bump_source]]` declared in config
//! 4. Conventional-commit inference (default)

use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{anyhow, Context as _};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::core::errors::Result;

/// Single bump decision from an external source. `release_unit` matches
/// the user-facing ReleaseUnit name; `bump` is one of `major | minor | patch`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BumpDecision {
    pub release_unit: String,
    pub bump: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// `version: 1` envelope for the JSON wire format.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BumpSourceFile {
    pub version: u32,
    #[serde(default)]
    pub decisions: Vec<BumpDecision>,
}

/// Default subprocess timeout when a `[[bump_source]]` doesn't specify one.
pub const DEFAULT_TIMEOUT_SEC: u64 = 60;

#[derive(Clone, Debug)]
pub enum BumpSourceInput {
    /// Read from a file path.
    File(std::path::PathBuf),
    /// Read from stdin (CI-only).
    Stdin,
    /// Run a shell command and parse its stdout.
    Command {
        cmd: String,
        timeout_sec: u64,
        /// Diagnostic label so log output ("running bump source `xyz`") is
        /// readable. Defaults to the command string.
        label: Option<String>,
    },
}

/// Parse + validate a JSON-shaped bump-source payload. Rejects anything
/// without `version: 1` so future format changes can't be silently misread
/// by older CLIs.
pub fn parse_bump_source(json: &str) -> Result<Vec<BumpDecision>> {
    let envelope: BumpSourceFile =
        serde_json::from_str(json).map_err(|e| anyhow!("bump-source JSON is malformed: {e}"))?;
    if envelope.version != 1 {
        return Err(anyhow!(
            "unsupported bump-source format version {} (this CLI only understands version 1)",
            envelope.version
        ));
    }
    for d in &envelope.decisions {
        validate_decision(d)?;
    }
    Ok(envelope.decisions)
}

fn validate_decision(d: &BumpDecision) -> Result<()> {
    if d.release_unit.trim().is_empty() {
        return Err(anyhow!(
            "bump-source decision has empty `release_unit` field"
        ));
    }
    match d.bump.as_str() {
        "major" | "minor" | "patch" => Ok(()),
        other => Err(anyhow!(
            "bump-source decision for `{}`: `bump` must be `major`, `minor`, or `patch` — got `{}`",
            d.release_unit,
            other
        )),
    }
}

/// Resolve one [`BumpSourceInput`] into a list of decisions. Errors include
/// the input descriptor so the user can find the problematic source.
pub fn collect(input: &BumpSourceInput) -> Result<Vec<BumpDecision>> {
    match input {
        BumpSourceInput::File(p) => {
            let body = std::fs::read_to_string(p)
                .with_context(|| format!("failed to read bump-source file `{}`", p.display()))?;
            parse_bump_source(&body)
                .with_context(|| format!("bump-source file `{}` is invalid", p.display()))
        }
        BumpSourceInput::Stdin => {
            let mut body = String::new();
            std::io::stdin()
                .read_to_string(&mut body)
                .context("failed to read bump-source from stdin")?;
            parse_bump_source(&body).context("bump-source from stdin is invalid")
        }
        BumpSourceInput::Command {
            cmd,
            timeout_sec,
            label,
        } => {
            let label = label.as_deref().unwrap_or(cmd);
            run_command(cmd, *timeout_sec, label)
        }
    }
}

fn run_command(cmd: &str, timeout_sec: u64, label: &str) -> Result<Vec<BumpDecision>> {
    use wait_timeout::ChildExt as _;

    info!("running bump source `{label}` (timeout {timeout_sec}s)");

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn bump-source command `{label}`"))?;

    // Drain stderr in a worker thread so a chatty subprocess can't block
    // its own pipe.
    let stderr = child.stderr.take().expect("piped above");
    let label_owned = label.to_string();
    let stderr_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(std::result::Result::ok) {
            info!("[{}] {}", label_owned, line);
        }
    });

    let timeout = Duration::from_secs(timeout_sec);
    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            // Timed out. Best-effort kill.
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_handle.join();
            return Err(anyhow!(
                "bump-source command `{label}` exceeded its {timeout_sec}s timeout — \
                 kill it or raise `timeout_sec` in [[bump_source]]"
            ));
        }
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_handle.join();
            return Err(anyhow!(
                "failed to wait for bump-source command `{label}`: {e}"
            ));
        }
    };

    let mut stdout_buf = String::new();
    if let Some(mut s) = child.stdout.take() {
        s.read_to_string(&mut stdout_buf)
            .with_context(|| format!("failed to read stdout from bump-source `{label}`"))?;
    }
    let _ = stderr_handle.join();

    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "<signal>".to_string());
        return Err(anyhow!(
            "bump-source command `{label}` failed with exit code {code}"
        ));
    }

    if stdout_buf.trim().is_empty() {
        warn!("bump source `{label}` produced no output — treating as zero decisions");
        return Ok(Vec::new());
    }

    parse_bump_source(&stdout_buf)
        .with_context(|| format!("bump-source `{label}` produced invalid output"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_v1() {
        let json = r#"{
            "version": 1,
            "decisions": [
                { "release_unit": "@org/foo", "bump": "minor",
                  "reason": "feature", "source": "test" }
            ]
        }"#;
        let d = parse_bump_source(json).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].release_unit, "@org/foo");
        assert_eq!(d[0].bump, "minor");
        assert_eq!(d[0].reason.as_deref(), Some("feature"));
    }

    #[test]
    fn rejects_unsupported_version() {
        let json = r#"{ "version": 2, "decisions": [] }"#;
        let err = parse_bump_source(json).unwrap_err();
        assert!(format!("{err:#}").contains("version 2"));
    }

    #[test]
    fn rejects_invalid_bump_string() {
        let json = r#"{ "version": 1,
          "decisions": [{ "release_unit": "x", "bump": "feature" }] }"#;
        let err = parse_bump_source(json).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("major"), "want major listed; got: {msg}");
        assert!(msg.contains("feature"), "want offending value; got: {msg}");
    }

    #[test]
    fn rejects_empty_project_name() {
        let json = r#"{ "version": 1,
          "decisions": [{ "release_unit": "  ", "bump": "patch" }] }"#;
        let err = parse_bump_source(json).unwrap_err();
        assert!(format!("{err:#}").contains("empty"));
    }

    #[test]
    fn omits_optional_fields_round_trip() {
        let d = BumpDecision {
            release_unit: "x".into(),
            bump: "patch".into(),
            reason: None,
            source: None,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("reason"), "absent reason must not serialize");
        assert!(!json.contains("source"), "absent source must not serialize");
    }

    #[test]
    fn collect_command_captures_stdout() {
        let input = BumpSourceInput::Command {
            cmd: r#"printf '{"version":1,"decisions":[{"release_unit":"x","bump":"minor"}]}'"#
                .to_string(),
            timeout_sec: 5,
            label: Some("test-echo".into()),
        };
        let d = collect(&input).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].release_unit, "x");
    }

    #[test]
    fn collect_command_times_out() {
        let input = BumpSourceInput::Command {
            cmd: "sleep 10".to_string(),
            timeout_sec: 1,
            label: Some("slow".into()),
        };
        let err = collect(&input).unwrap_err();
        assert!(
            format!("{err:#}").contains("timeout"),
            "want timeout error, got: {err:#}"
        );
    }

    #[test]
    fn collect_command_propagates_nonzero_exit() {
        let input = BumpSourceInput::Command {
            cmd: "exit 17".to_string(),
            timeout_sec: 5,
            label: Some("fail".into()),
        };
        let err = collect(&input).unwrap_err();
        assert!(format!("{err:#}").contains("17"));
    }
}
