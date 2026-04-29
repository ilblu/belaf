//! `ExternalVersionerRewriter` — drives a user-supplied tool (fastlane,
//! gradle plugin, buf, custom shell) that owns the version source.
//!
//! Phase E of `BELAF_MASTER_PLAN.md`. Belaf does **no** format
//! introspection — it shells out to `read_command` to learn the
//! current version, runs `write_command` with `{version}`/`{bump}`/
//! `{name}` substitutions to perform the bump, then re-runs
//! `read_command` to confirm the move. On mismatch or non-zero exit
//! it surfaces the captured stdout+stderr.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use thiserror::Error;
use tracing::info;
use wait_timeout::ChildExt as _;

use crate::core::git::repository::Repository;
use crate::core::release_unit::ExternalVersioner;

#[derive(Debug, Error)]
pub enum ExternalVersionerError {
    #[error("external versioner `{tool}` failed to spawn `{stage}` command: {source}")]
    Spawn {
        tool: String,
        stage: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("external versioner `{tool}` `{stage}` command exceeded its {timeout_sec}s timeout")]
    Timeout {
        tool: String,
        stage: &'static str,
        timeout_sec: u64,
    },

    #[error(
        "external versioner `{tool}` `{stage}` command exited with status {code} — stdout: <{stdout}>, stderr: <{stderr}>"
    )]
    NonZeroExit {
        tool: String,
        stage: &'static str,
        code: String,
        stdout: String,
        stderr: String,
    },

    #[error(
        "external versioner `{tool}` `{stage}` returned empty version (stdout was: <{stdout}>)"
    )]
    EmptyVersion {
        tool: String,
        stage: &'static str,
        stdout: String,
    },

    #[error(
        "external versioner `{tool}`: write succeeded but re-read returned `{actual}`, expected `{expected}`"
    )]
    PostWriteMismatch {
        tool: String,
        actual: String,
        expected: String,
    },

    #[error("error draining stderr/stdout from `{tool}`: {source}")]
    PipeIo {
        tool: String,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, ExternalVersionerError>;

/// Bump-type info passed in to `write_command` substitutions and as
/// `BELAF_BUMP_TYPE` env. Stable wire-format keys aligned with the
/// `KnownBumpType` enum.
#[derive(Debug, Clone, Copy)]
pub enum BumpKind {
    Major,
    Minor,
    Patch,
    Prerelease,
}

impl BumpKind {
    pub fn wire_key(&self) -> &'static str {
        match self {
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
            Self::Prerelease => "prerelease",
        }
    }
}

/// Read the current version via `ext.read_command`. Returns the raw
/// stdout of the command, trimmed of surrounding whitespace.
pub fn read_current(ext: &ExternalVersioner, repo: &Repository) -> Result<String> {
    let stdout = run_command(ext, repo, &ext.read_command, "read", &HashMap::new())?;
    let trimmed = stdout.trim().to_string();
    if trimmed.is_empty() {
        return Err(ExternalVersionerError::EmptyVersion {
            tool: ext.tool.clone(),
            stage: "read",
            stdout,
        });
    }
    Ok(trimmed)
}

/// Run `ext.write_command` with the substitution context, then re-run
/// `read_command` and verify the version moved to `new_version`.
pub fn write_and_verify(
    ext: &ExternalVersioner,
    repo: &Repository,
    unit_name: &str,
    new_version: &str,
    bump: BumpKind,
) -> Result<()> {
    // Idempotency check first — if already at target, do nothing.
    let before = read_current(ext, repo).ok();
    if before.as_deref() == Some(new_version) {
        return Ok(());
    }

    // Build the substitution map used both as command-template
    // placeholders AND as BELAF_*-prefixed env vars on the spawned
    // process.
    let mut subs = HashMap::new();
    subs.insert("version".to_string(), new_version.to_string());
    subs.insert("bump".to_string(), bump.wire_key().to_string());
    subs.insert("name".to_string(), unit_name.to_string());

    let cmd = apply_substitutions(&ext.write_command, &subs);
    let _stdout = run_command(ext, repo, &cmd, "write", &subs)?;

    // Re-read to confirm the move actually happened.
    let after = read_current(ext, repo)?;
    if after != new_version {
        return Err(ExternalVersionerError::PostWriteMismatch {
            tool: ext.tool.clone(),
            actual: after,
            expected: new_version.to_string(),
        });
    }
    Ok(())
}

fn apply_substitutions(template: &str, subs: &HashMap<String, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in subs {
        let placeholder = format!("{{{k}}}");
        out = out.replace(&placeholder, v);
    }
    out
}

fn run_command(
    ext: &ExternalVersioner,
    repo: &Repository,
    shell_cmd: &str,
    stage: &'static str,
    template_subs: &HashMap<String, String>,
) -> Result<String> {
    info!(
        "external_versioner[{tool}] running `{stage}` (timeout {to}s)",
        tool = ext.tool,
        stage = stage,
        to = ext.timeout_sec,
    );

    let cwd: PathBuf = match &ext.cwd {
        Some(p) => repo.resolve_workdir(p),
        None => repo.resolve_workdir(&crate::core::git::repository::RepoPathBuf::new(b"")),
    };

    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(shell_cmd)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // BELAF_*-prefixed env. {version}/{bump}/{name} from template_subs
    // surface as BELAF_VERSION_NEW / BELAF_BUMP_TYPE / BELAF_UNIT_NAME.
    if let Some(v) = template_subs.get("version") {
        command.env("BELAF_VERSION_NEW", v);
    }
    if let Some(b) = template_subs.get("bump") {
        command.env("BELAF_BUMP_TYPE", b);
    }
    if let Some(n) = template_subs.get("name") {
        command.env("BELAF_UNIT_NAME", n);
    }
    command.env("BELAF_EXTERNAL_TOOL", &ext.tool);
    // User-defined env vars take precedence (last applied wins in
    // std::process::Command).
    for (k, v) in &ext.env {
        command.env(k, v);
    }

    let mut child = command.spawn().map_err(|e| ExternalVersionerError::Spawn {
        tool: ext.tool.clone(),
        stage,
        source: e,
    })?;

    // Drain stderr in a worker so a chatty subprocess can't block its
    // own pipe.
    let stderr_pipe = child.stderr.take().expect("piped above");
    let label = ext.tool.clone();
    let stderr_handle = std::thread::spawn(move || -> std::io::Result<String> {
        let reader = BufReader::new(stderr_pipe);
        let mut all = String::new();
        for line in reader.lines() {
            let line = line?;
            info!("[{}] {}", label, line);
            all.push_str(&line);
            all.push('\n');
        }
        Ok(all)
    });

    let timeout = Duration::from_secs(ext.timeout_sec);
    let status = match child.wait_timeout(timeout) {
        Ok(Some(s)) => s,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_handle.join();
            return Err(ExternalVersionerError::Timeout {
                tool: ext.tool.clone(),
                stage,
                timeout_sec: ext.timeout_sec,
            });
        }
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_handle.join();
            return Err(ExternalVersionerError::Spawn {
                tool: ext.tool.clone(),
                stage,
                source: e,
            });
        }
    };

    let mut stdout_buf = String::new();
    if let Some(mut s) = child.stdout.take() {
        s.read_to_string(&mut stdout_buf)
            .map_err(|e| ExternalVersionerError::PipeIo {
                tool: ext.tool.clone(),
                source: e,
            })?;
    }
    let stderr_collected = stderr_handle
        .join()
        .unwrap_or_else(|_| Ok(String::new()))
        .unwrap_or_default();

    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "<signal>".to_string());
        return Err(ExternalVersionerError::NonZeroExit {
            tool: ext.tool.clone(),
            stage,
            code,
            stdout: stdout_buf.trim().to_string(),
            stderr: stderr_collected.trim().to_string(),
        });
    }

    Ok(stdout_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution_replaces_all_placeholders() {
        let mut subs = HashMap::new();
        subs.insert("version".to_string(), "1.2.3".to_string());
        subs.insert("bump".to_string(), "minor".to_string());
        subs.insert("name".to_string(), "aura".to_string());

        let out = apply_substitutions("echo bumping {name} ({bump}) to {version}", &subs);
        assert_eq!(out, "echo bumping aura (minor) to 1.2.3");
    }

    #[test]
    fn substitution_leaves_unknown_placeholders_alone() {
        let subs = HashMap::new();
        let out = apply_substitutions("echo {unknown}", &subs);
        assert_eq!(out, "echo {unknown}");
    }

    #[test]
    fn bump_kind_wire_keys() {
        assert_eq!(BumpKind::Major.wire_key(), "major");
        assert_eq!(BumpKind::Minor.wire_key(), "minor");
        assert_eq!(BumpKind::Patch.wire_key(), "patch");
        assert_eq!(BumpKind::Prerelease.wire_key(), "prerelease");
    }
}
