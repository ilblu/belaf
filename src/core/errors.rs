// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Error handling and user-facing diagnostic rendering.
//!
//! This module wraps `anyhow::Error` and offers two layers:
//!
//! * `Result`/`Error` re-exports + `AnnotatedReport` (a context type that
//!   carries `notes: Vec<String>`). Used pervasively via the `atry!` and
//!   `a_ok_or!` macros below.
//! * A user-facing `display_diagnostic(&Error)` that renders a structured,
//!   `rustc`-style diagnostic via `annotate-snippets`. Caused-by chain
//!   becomes context lines; `AnnotatedReport.notes` and downcasts of typed
//!   errors (`ApiError`, `DirtyRepositoryError`, etc.) become `help:`
//!   groups underneath.

use std::io::{stderr, IsTerminal};
use std::sync::atomic::{AtomicBool, Ordering};

use annotate_snippets::{Group, Level, Renderer};
use thiserror::Error as ThisError;

/// Global "no color" override. `main` sets this to `true` when the user
/// passes `--no-color`. `display_diagnostic` honors it alongside the
/// standard `NO_COLOR` env var.
static FORCE_NO_COLOR: AtomicBool = AtomicBool::new(false);

pub fn set_no_color(value: bool) {
    FORCE_NO_COLOR.store(value, Ordering::Relaxed);
}

pub use anyhow::Error;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Default, ThisError)]
#[error("{message}")]
pub struct AnnotatedReport {
    message: String,
    notes: Vec<String>,
}

impl AnnotatedReport {
    pub fn set_message(&mut self, m: String) {
        self.message = m;
    }

    pub fn add_note(&mut self, n: String) {
        self.notes.push(n);
    }

    pub fn notes(&self) -> &[String] {
        &self.notes[..]
    }
}

#[doc(hidden)]
pub use anyhow::Context;

/// Annotated try — like `?`, but with the ability to add a wrapping message
/// and `(note "...")` annotations to the resulting error. Notes are surfaced
/// as `help:` lines in the final diagnostic.
#[macro_export]
macro_rules! atry {
    (@aa $ar:ident [ $($inner:tt)+ ] ) => {
        $ar.set_message(format!($($inner)+));
    };
    (@aa $ar:ident ( note $($inner:tt)+ ) ) => {
        $ar.add_note(format!($($inner)+));
    };
    ($op:expr ; $( $annotation:tt )+) => {{
        use $crate::core::errors::Context;
        $op.with_context(|| {
            let mut ar = $crate::core::errors::AnnotatedReport::default();
            $(
                atry!(@aa ar $annotation);
            )+
            ar
        })?
    }};
}

/// Annotated `ok_or` — like `Option::ok_or_else()?`, but with the ability to
/// add wrapping messages and notes.
#[macro_export]
macro_rules! a_ok_or {
    (@aa $ar:ident [ $($inner:tt)+ ] ) => {
        $ar.set_message(format!($($inner)+));
    };
    (@aa $ar:ident ( note $($inner:tt)+ ) ) => {
        $ar.add_note(format!($($inner)+));
    };
    ($option:expr ; $( $annotation:tt )+) => {{
        use $crate::atry;
        $option.ok_or_else(|| {
            let mut ar = $crate::core::errors::AnnotatedReport::default();
            $(
                atry!(@aa ar $annotation);
            )+
            ar
        })?
    }};
}

// ---------------------------------------------------------------------------
// Diagnostic rendering
// ---------------------------------------------------------------------------

/// Render `error` as a `rustc`-style diagnostic to stderr.
///
/// Color is decided by `use_color()`: respects `NO_COLOR`, the `--no-color`
/// flag (via `owo_colors`), and stderr-is-a-TTY.
pub fn display_diagnostic(error: &Error) {
    let rendered = render_diagnostic(error, use_color());
    eprintln!();
    eprintln!("{}", rendered);
}

/// Convenience used by `cmd::*::run() -> Result<i32>` callers that want to
/// turn an error into an exit code while printing it.
pub fn report(r: Result<i32>) -> i32 {
    match r {
        Ok(c) => c,
        Err(e) => {
            display_diagnostic(&e);
            1
        }
    }
}

/// Plain (no-color) renderer for tests and snapshot assertions.
#[doc(hidden)]
pub fn display_diagnostic_to_string(error: &Error) -> String {
    render_diagnostic(error, false)
}

fn render_diagnostic(error: &Error, color: bool) -> String {
    // Materialize all strings first so the borrow checker is happy with
    // annotate-snippets' lifetime-coupled types.
    let primary_msg = error.to_string();
    let cause_lines: Vec<String> = error.chain().skip(1).map(|c| format!("× {}", c)).collect();
    let notes: Vec<String> = collect_notes(error);
    let typed_hints = derive_typed_hints(error);

    // Title.element() returns a Group — collapse causes via fold so the
    // primary group ends up as one Group<'_> regardless of cause count.
    let primary_group: Group<'_> = cause_lines.iter().fold(
        Group::with_title(Level::ERROR.primary_title(primary_msg.as_str())),
        |g, cl| g.element(Level::NOTE.message(cl.as_str())),
    );

    let mut report: Vec<Group<'_>> = vec![primary_group];
    for note in &notes {
        report.push(Group::with_title(
            Level::HELP.secondary_title(note.as_str()),
        ));
    }
    for hint in &typed_hints {
        report.push(Group::with_title(
            Level::HELP.secondary_title(hint.as_str()),
        ));
    }

    let renderer = if color {
        Renderer::styled()
    } else {
        Renderer::plain()
    };
    renderer.render(&report)
}

/// Collect notes from any `AnnotatedReport` reachable from `error`.
///
/// `error.downcast_ref` finds the outermost anyhow context (which lives
/// outside the `dyn StdError` source chain); `error.chain()` walks the
/// inner sources. We dedup at the end because anyhow can sometimes surface
/// the same layer through both paths.
fn collect_notes(error: &Error) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(ann) = error.downcast_ref::<AnnotatedReport>() {
        notes.extend(ann.notes().iter().cloned());
    }
    for layer in error.chain() {
        if let Some(ann) = layer.downcast_ref::<AnnotatedReport>() {
            notes.extend(ann.notes().iter().cloned());
        }
    }
    notes.dedup();
    notes
}

fn use_color() -> bool {
    // The `--no-color` CLI flag (set via `set_no_color` in main).
    if FORCE_NO_COLOR.load(Ordering::Relaxed) {
        return false;
    }
    // Standard NO_COLOR env (https://no-color.org) — universal opt-out.
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    stderr().is_terminal()
}

/// Walk every layer of the error (including the outermost anyhow context,
/// which is *not* in the `dyn StdError` source chain) and emit hints for
/// every recognised typed error along the way.
fn derive_typed_hints(error: &Error) -> Vec<String> {
    use crate::core::api::ApiError;
    use crate::core::git::repository::{BareRepositoryError, DirtyRepositoryError};

    fn hint_for_api(api: &ApiError) -> Option<String> {
        match api {
            ApiError::RateLimited { retry_after_secs } => Some(format!(
                "re-run after {} second{}",
                retry_after_secs,
                if *retry_after_secs == 1 { "" } else { "s" }
            )),
            ApiError::Unauthorized => Some("run `belaf install` to re-authenticate".to_string()),
            ApiError::ApiResponse { status, .. } if (500..600).contains(status) => {
                Some("transient server error — try again in a moment".to_string())
            }
            ApiError::DeviceCodeExpired | ApiError::DeviceCodeDenied => {
                Some("re-run `belaf install` to start a fresh device-flow".to_string())
            }
            ApiError::LimitExceeded { upgrade_url, .. } if !upgrade_url.is_empty() => {
                Some(format!("upgrade your plan: {}", upgrade_url))
            }
            ApiError::LimitExceeded { .. } => {
                Some("upgrade your plan to remove the repository limit".to_string())
            }
            _ => None,
        }
    }

    let mut hints = Vec::new();

    // anyhow context types: wrapped, only reachable via Error::downcast_ref.
    if let Some(api) = error.downcast_ref::<ApiError>() {
        if let Some(h) = hint_for_api(api) {
            hints.push(h);
        }
    }
    if error.downcast_ref::<DirtyRepositoryError>().is_some() {
        hints.push("commit or stash your changes, or pass `--force` to override".to_string());
    }
    if error.downcast_ref::<BareRepositoryError>().is_some() {
        hints.push("belaf must run inside a working tree, not a bare repository".to_string());
    }

    // Plus every layer in the standard source() chain.
    for layer in error.chain() {
        if let Some(api) = layer.downcast_ref::<ApiError>() {
            if let Some(h) = hint_for_api(api) {
                hints.push(h);
            }
        }
        if layer.downcast_ref::<DirtyRepositoryError>().is_some() {
            hints.push("commit or stash your changes, or pass `--force` to override".to_string());
        }
        if layer.downcast_ref::<BareRepositoryError>().is_some() {
            hints.push("belaf must run inside a working tree, not a bare repository".to_string());
        }
    }

    // Dedup: anyhow's `downcast_ref` and `chain()` may both find the same
    // layer (anyhow wraps the inner error itself in some cases).
    hints.dedup();
    hints
}
