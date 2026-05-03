//! Snapshot tests for the user-facing error renderer.
//!
//! These pin the rendered output of `display_diagnostic_to_string` (which
//! always uses the plain, no-color renderer) against committed `.snap` files
//! under `tests/snapshots/`. Any change to the rendered shape — layout, error
//! titles, hint wording, chain order — needs to be reviewed via `cargo insta
//! review`.

use belaf::core::api::ApiError;
use belaf::core::errors::{display_diagnostic_to_string, AnnotatedReport};
use belaf::core::git::repository::{BareRepositoryError, DirtyRepositoryError, RepoPathBuf};

#[test]
fn dirty_repository_renders_with_hint() {
    let dirty = DirtyRepositoryError(RepoPathBuf::new(b".gitignore"));
    let err = anyhow::Error::new(dirty);
    insta::assert_snapshot!(display_diagnostic_to_string(&err));
}

#[test]
fn chain_with_annotated_notes() {
    let mut ann = AnnotatedReport::default();
    ann.set_message("could not load embedded default configuration".into());
    ann.add_note(
        "this is a packaging bug — please report at https://github.com/ilblu/belaf/issues".into(),
    );
    let err = anyhow::Error::new(ann).context("init step failed");
    insta::assert_snapshot!(display_diagnostic_to_string(&err));
}

#[test]
fn api_rate_limit_renders_with_retry_hint() {
    let err = anyhow::Error::new(ApiError::RateLimited {
        retry_after_secs: 12,
    });
    insta::assert_snapshot!(display_diagnostic_to_string(&err));
}

#[test]
fn api_unauthorized_renders_with_install_hint() {
    let err = anyhow::Error::new(ApiError::Unauthorized);
    insta::assert_snapshot!(display_diagnostic_to_string(&err));
}

#[test]
fn bare_repository_renders_with_hint() {
    let err = anyhow::Error::new(BareRepositoryError);
    insta::assert_snapshot!(display_diagnostic_to_string(&err));
}

#[test]
fn simple_error_no_hints_no_notes() {
    let err = anyhow::anyhow!("something broke");
    insta::assert_snapshot!(display_diagnostic_to_string(&err));
}
