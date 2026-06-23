//! `belaf check` (F8) — validate commit labels against the release model.
//!
//! Two label-only checks (this command never gates the cascade — a failing
//! check blocks a *mislabeled* PR, never a correct release):
//!   1. a commit's conventional **scope** names a known release unit
//!      (`deploy` or `internal`);
//!   2. the commit's **binary-affecting changed paths** fall within that
//!      unit's dependency closure (its own crate, or an internal crate it
//!      cascades through).
//!
//! Modes: `--message <MSG>` (commit-msg hook — scope only, no diff yet),
//! `--range <A>..<B>` (CI), or neither (defaults to `HEAD`). `--ci` turns
//! violations into a hard failure; locally they are warnings.

use anyhow::Result;
use owo_colors::OwoColorize;

use crate::core::bump::{extract_scope, ScopeMatcher};
use crate::core::git::repository::{is_binary_affecting, CommitId};
use crate::core::graph::ReleaseUnitGraph;
use crate::core::resolved_release_unit::UnitKind;
use crate::core::session::{AppBuilder, AppSession};

pub fn run(message: Option<String>, range: Option<String>, ci: bool) -> Result<i32> {
    let sess = AppBuilder::new()?.initialize()?;
    let graph = sess.graph();

    // Known scopes = deploy + internal unit names (ignore units excluded).
    let unit_names: Vec<String> = graph
        .projects_slice()
        .iter()
        .filter(|u| u.kind != UnitKind::Ignore)
        .map(|u| u.user_facing_name.clone())
        .collect();
    let scope_matcher = ScopeMatcher::from_config(sess.commit_attribution());

    let mut violations: Vec<String> = Vec::new();

    if let Some(msg) = message.as_deref() {
        // commit-msg hook: only the message is available → scope check only.
        check_scope(
            "(message)",
            msg,
            &unit_names,
            &scope_matcher,
            &mut violations,
        );
    } else {
        let range = range.unwrap_or_else(|| "HEAD".to_string());
        let commits = sess.repo.commits_in_range(&range)?;
        for cid in commits {
            check_commit(
                &sess,
                graph,
                cid,
                &unit_names,
                &scope_matcher,
                &mut violations,
            )?;
        }
    }

    if violations.is_empty() {
        println!("{} commit labels OK", "✓".green());
        return Ok(0);
    }

    eprintln!(
        "{} belaf check found {} issue(s):",
        "✗".red(),
        violations.len()
    );
    for v in &violations {
        eprintln!("  - {v}");
    }
    if ci {
        Ok(1) // hard-fail in CI
    } else {
        eprintln!("(warning only — re-run with --ci to fail the build)");
        Ok(0)
    }
}

fn short(cid: CommitId) -> String {
    cid.to_string().chars().take(8).collect()
}

/// Validate that a commit message's conventional scope names a known unit.
/// Returns false (and records a violation) on a mislabeled scope. A commit
/// with no scope is fine — there's nothing to mislabel.
fn check_scope(
    label: &str,
    message: &str,
    unit_names: &[String],
    matcher: &ScopeMatcher,
    out: &mut Vec<String>,
) -> bool {
    let Some(scope) = extract_scope(message) else {
        return true;
    };
    if matcher.find_matching_project(&scope, unit_names).is_none() {
        out.push(format!(
            "{label}: scope `{scope}` is not a known release unit (deploy/internal)"
        ));
        false
    } else {
        true
    }
}

/// Full per-commit validation: scope membership + path/closure consistency.
fn check_commit(
    sess: &AppSession,
    graph: &ReleaseUnitGraph,
    cid: CommitId,
    unit_names: &[String],
    matcher: &ScopeMatcher,
    out: &mut Vec<String>,
) -> Result<()> {
    let commit = sess.repo.get_commit_details(cid)?;
    let label = short(cid);

    let Some(scope) = extract_scope(&commit.message) else {
        return Ok(());
    };
    let Some(unit_name) = matcher.find_matching_project(&scope, unit_names) else {
        out.push(format!(
            "{label}: scope `{scope}` is not a known release unit (deploy/internal)"
        ));
        return Ok(());
    };

    // Path consistency: every binary-affecting changed path must fall within
    // the scoped unit's dependency closure (own crate, or a cascaded internal).
    let Some(uid) = graph.lookup_ident(unit_name) else {
        return Ok(());
    };
    let closure = graph.closure(uid);
    for p in sess.repo.commit_changed_paths(cid)? {
        let esc = p.escaped();
        if !is_binary_affecting(esc.as_bytes(), sess.binary_affecting()) {
            continue;
        }
        let in_closure = closure
            .iter()
            .any(|&m| graph.lookup(m).repo_paths.repo_path_matches(p.as_ref()));
        if !in_closure {
            out.push(format!(
                "{label}: scoped `{scope}` but changed `{esc}` is outside {unit_name}'s closure"
            ));
        }
    }
    Ok(())
}
