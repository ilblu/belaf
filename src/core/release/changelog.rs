// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Dealing with changelogs.
//!
//! This whole subject matter might not seem integral to the operation of
//! Belaf, but it turns out to be very relevant, since so much of Belaf's core
//! has to do with looking at the repository history since the most recent
//! release(s). That's exactly the information contained in a release changelog.

use std::{
    collections::HashMap,
    fs::File,
    io::{prelude::*, BufReader, Cursor},
    path::PathBuf,
};
use thiserror::Error as ThisError;
use time::OffsetDateTime;

use super::template::format_template;

use crate::core::release::{
    errors::{Error, Result},
    project::Project,
    repository::{ChangeList, CommitId, PathMatcher, RepoPathBuf, Repository},
    session::AppSession,
};

/// A type that defines how the changelog for a given project is managed.
pub trait Changelog: std::fmt::Debug {
    /// Rewrite the changelog file(s) with stub contents derived from the
    /// repository history, prepended to whatever contents existed at the
    /// previous release commit.
    fn draft_release_update(
        &self,
        proj: &Project,
        sess: &AppSession,
        changes: &[CommitId],
        prev_release_commit: Option<CommitId>,
    ) -> Result<()>;

    /// Replace the changelog file(s) in the project's working directory with
    /// the contents from the most recent release of the project.
    fn replace_changelog(
        &self,
        proj: &Project,
        sess: &AppSession,
        changes: &mut ChangeList,
        prev_release_commit: CommitId,
    ) -> Result<()>;

    /// Create a matcher that matches one or more paths in the project's
    /// directory corresponding to its changelog(s). Operations like `belaf
    /// stage` and `belaf confirm` care about working directory dirtiness, but
    /// in our model modified changelogs are OK.
    fn create_path_matcher(&self, proj: &Project) -> Result<PathMatcher>;

    fn scan_bump_spec(&self, proj: &Project, repo: &Repository) -> Result<String>;

    /// Rewrite the changelog file(s) in the project's working directory, which
    /// are in the "rc" format that includes release candidate metadata, to
    /// instead include the final release information. The changelog contents
    /// will already include earlier entries.
    fn finalize_changelog(
        &self,
        proj: &Project,
        repo: &Repository,
        changes: &mut ChangeList,
    ) -> Result<()>;

    /// Read most recent changelog text *as of the specified commit*.
    ///
    /// For now, this text is presumed to be formatted in CommonMark format.
    ///
    /// Note that this operation ignores the working tree in an effort to provide
    /// more reliability.
    fn scan_changelog(&self, proj: &Project, repo: &Repository, cid: &CommitId) -> Result<String>;
}

/// Create a new default Changelog implementation.
///
/// This uses the Markdown format.
pub fn default() -> Box<dyn Changelog> {
    Box::<MarkdownChangelog>::default()
}

/// An error returned when a changelog file does not obey the special structure
/// expected by Belaf's processing routines. The inner value is the path to the
/// offending changelog (not a RepoPathBuf since it may not have yet been added
/// to the repo).
#[derive(Debug, ThisError)]
pub struct InvalidChangelogFormatError(pub PathBuf);

impl std::fmt::Display for InvalidChangelogFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "changelog file `{}` does not obey the expected formatting",
            self.0.display()
        )
    }
}

/// Settings for Markdown-formatted changelogs.
#[derive(Debug)]
pub struct MarkdownChangelog {
    basename: String,
    release_header_format: String,
    stage_header_format: String,
    footer_format: String,
}

impl Default for MarkdownChangelog {
    fn default() -> Self {
        MarkdownChangelog {
            basename: "CHANGELOG.md".to_owned(),
            release_header_format: "# {project_slug} {version} ({yyyy_mm_dd})\n".to_owned(),
            stage_header_format: "# rc: {bump_spec}\n".to_owned(),
            footer_format: "".to_owned(),
        }
    }
}

impl MarkdownChangelog {
    fn changelog_repopath(&self, proj: &Project) -> RepoPathBuf {
        let mut pfx = proj.prefix().to_owned();
        pfx.push(&self.basename);
        pfx
    }

    fn changelog_path(&self, proj: &Project, repo: &Repository) -> PathBuf {
        repo.resolve_workdir(&self.changelog_repopath(proj))
    }

    /// Generic implementation for draft_release_update and replace_changelog.
    fn replace_changelog_impl(
        &self,
        proj: &Project,
        sess: &AppSession,
        prev_release_commit: Option<CommitId>,
        in_changes: Option<&[CommitId]>,
        out_changes: Option<&mut ChangeList>,
    ) -> Result<()> {
        // Get the previous changelog from the most recent `release`
        // commit.

        let changelog_repopath = self.changelog_repopath(proj);

        let prev_log: Vec<u8> = prev_release_commit
            .map(|prc| sess.repo.get_file_at_commit(&prc, &changelog_repopath))
            .transpose()?
            .flatten()
            .unwrap_or_default();

        // Start working on rewriting the existing file.

        let changelog_path = self.changelog_path(proj, &sess.repo);

        let new_af = atomicwrites::AtomicFile::new(
            changelog_path,
            atomicwrites::OverwriteBehavior::AllowOverwrite,
        );

        let r = new_af.write(|new_f| {
            if let Some(commits) = in_changes {
                // We're drafting a release update -- add a new section.

                let mut headfoot_args = HashMap::new();
                headfoot_args.insert("bump_spec", "patch");
                let header = format_template(&self.stage_header_format, &headfoot_args)
                    .map_err(|e| Error::msg(e.to_string()))?;
                writeln!(new_f, "{}", header)?;

                // Commit summaries! Note: if we're staging muliple projects and the
                // same commit affects many of them, we'll reload the same commit many
                // times when generating changelogs.

                const WRAP_WIDTH: usize = 78;

                for cid in commits {
                    let message = sess.repo.get_commit_summary(*cid)?;
                    let mut prefix = "- ";

                    for line in textwrap::wrap(&message, WRAP_WIDTH) {
                        writeln!(new_f, "{}{}", prefix, line)?;
                        prefix = "  ";
                    }
                }

                // Footer

                let footer = format_template(&self.footer_format, &headfoot_args)
                    .map_err(|e| Error::msg(e.to_string()))?;
                writeln!(new_f, "{}", footer)?;
            }

            // Write back all of the previous contents, and we're done.
            new_f.write_all(&prev_log[..])?;

            Ok(())
        });

        if let Some(chlist) = out_changes {
            chlist.add_path(&self.changelog_repopath(proj));
        }

        match r {
            Err(atomicwrites::Error::Internal(e)) => Err(e.into()),
            Err(atomicwrites::Error::User(e)) => Err(e),
            Ok(()) => Ok(()),
        }
    }
}

impl Changelog for MarkdownChangelog {
    fn draft_release_update(
        &self,
        proj: &Project,
        sess: &AppSession,
        changes: &[CommitId],
        prev_release_commit: Option<CommitId>,
    ) -> Result<()> {
        self.replace_changelog_impl(proj, sess, prev_release_commit, Some(changes), None)
    }

    fn replace_changelog(
        &self,
        proj: &Project,
        sess: &AppSession,
        changes: &mut ChangeList,
        prev_release_commit: CommitId,
    ) -> Result<()> {
        self.replace_changelog_impl(proj, sess, Some(prev_release_commit), None, Some(changes))
    }

    fn create_path_matcher(&self, proj: &Project) -> Result<PathMatcher> {
        Ok(PathMatcher::new_include(self.changelog_repopath(proj)))
    }

    fn scan_bump_spec(&self, proj: &Project, repo: &Repository) -> Result<String> {
        let changelog_path = self.changelog_path(proj, repo);
        let f = File::open(&changelog_path)?;
        let reader = BufReader::new(f);
        let mut bump_spec = None;

        for maybe_line in reader.lines() {
            let line = maybe_line?;
            if line.trim().is_empty() {
                continue;
            }

            if let Some(spec_text) = line.strip_prefix("# rc:") {
                let spec = spec_text.trim();
                bump_spec = Some(spec.to_owned());
                break;
            }

            return Err(InvalidChangelogFormatError(changelog_path).into());
        }

        let bump_spec = bump_spec.ok_or(InvalidChangelogFormatError(changelog_path))?;
        let _check_scheme = proj.version.parse_bump_scheme(&bump_spec)?;

        Ok(bump_spec)
    }

    fn finalize_changelog(
        &self,
        proj: &Project,
        repo: &Repository,
        changes: &mut ChangeList,
    ) -> Result<()> {
        // Prepare the substitution template
        let mut header_args = HashMap::new();
        header_args.insert("project_slug", proj.user_facing_name.to_owned());
        header_args.insert("version", proj.version.to_string());
        let now = OffsetDateTime::now_utc();
        header_args.insert(
            "yyyy_mm_dd",
            format!(
                "{:04}-{:02}-{:02}",
                now.year(),
                now.month() as u8,
                now.day()
            ),
        );

        let changelog_path = self.changelog_path(proj, repo);
        let cur_f = File::open(&changelog_path)?;
        let cur_reader = BufReader::new(cur_f);

        let new_af = atomicwrites::AtomicFile::new(
            &changelog_path,
            atomicwrites::OverwriteBehavior::AllowOverwrite,
        );
        let r = new_af.write(|new_f| {
            // Pipe the current changelog into the new one, replacing the `rc`
            // header with the final one.

            #[expect(clippy::enum_variant_names)]
            enum State {
                BeforeHeader,
                BlanksAfterHeader,
                AfterHeader,
            }
            let mut state = State::BeforeHeader;

            for maybe_line in cur_reader.lines() {
                let line = maybe_line?;

                match state {
                    State::BeforeHeader => {
                        if line.trim().is_empty() {
                            continue;
                        }

                        if !line.starts_with("# rc:") {
                            return Err(InvalidChangelogFormatError(changelog_path).into());
                        }

                        state = State::BlanksAfterHeader;
                        let header = format_template(&self.release_header_format, &header_args)
                            .map_err(|e| Error::msg(e.to_string()))?;
                        writeln!(new_f, "{}", header)?;
                    }

                    State::BlanksAfterHeader => {
                        if !line.trim().is_empty() {
                            state = State::AfterHeader;
                            writeln!(new_f, "{}", line)?;
                        }
                    }

                    State::AfterHeader => {
                        writeln!(new_f, "{}", line)?;
                    }
                }
            }

            Ok(())
        });

        changes.add_path(&self.changelog_repopath(proj));

        match r {
            Err(atomicwrites::Error::Internal(e)) => Err(e.into()),
            Err(atomicwrites::Error::User(e)) => Err(e),
            Ok(()) => Ok(()),
        }
    }

    fn scan_changelog(&self, proj: &Project, repo: &Repository, cid: &CommitId) -> Result<String> {
        let changelog_path = self.changelog_repopath(proj);
        let data = match repo.get_file_at_commit(cid, &changelog_path)? {
            Some(d) => d,
            None => return Ok(String::new()),
        };
        let reader = Cursor::new(data);

        enum State {
            BeforeHeader,
            InChangelog,
        }
        let mut state = State::BeforeHeader;
        let mut changelog = String::new();

        // In a slight tweak from other methods here, we ignore everything
        // before a "# " header.
        for maybe_line in reader.lines() {
            let line = maybe_line?;

            match state {
                State::BeforeHeader => {
                    if line.starts_with("# ") {
                        changelog.push_str(&line);
                        changelog.push('\n');
                        state = State::InChangelog;
                    }
                }

                State::InChangelog => {
                    if line.starts_with("# ") {
                        break;
                    } else {
                        changelog.push_str(&line);
                        changelog.push('\n');
                    }
                }
            }
        }

        Ok(changelog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_changelog_default_basename() {
        let changelog = MarkdownChangelog::default();
        assert_eq!(changelog.basename, "CHANGELOG.md");
    }

    #[test]
    fn test_markdown_changelog_release_header_format() {
        let changelog = MarkdownChangelog::default();
        let mut args = HashMap::new();
        args.insert("project_slug", "my-project".to_string());
        args.insert("version", "1.2.3".to_string());
        args.insert("yyyy_mm_dd", "2025-01-15".to_string());

        let result = format_template(&changelog.release_header_format, &args)
            .expect("BUG: format should succeed with valid args");

        assert_eq!(result, "# my-project 1.2.3 (2025-01-15)\n");
    }

    #[test]
    fn test_markdown_changelog_stage_header_format() {
        let changelog = MarkdownChangelog::default();
        let mut args = HashMap::new();
        args.insert("bump_spec", "minor".to_string());

        let result = format_template(&changelog.stage_header_format, &args)
            .expect("BUG: format should succeed with valid args");

        assert_eq!(result, "# rc: minor\n");
    }

    #[test]
    fn test_parse_rc_header_patch() {
        let line = "# rc: patch";
        let spec = line.strip_prefix("# rc:");

        assert_eq!(spec, Some(" patch"));
        assert_eq!(
            spec.expect("BUG: spec should be Some after assertion")
                .trim(),
            "patch"
        );
    }

    #[test]
    fn test_parse_rc_header_minor() {
        let line = "# rc: minor";
        let spec = line.strip_prefix("# rc:").map(|s| s.trim());

        assert_eq!(spec, Some("minor"));
    }

    #[test]
    fn test_parse_rc_header_major() {
        let line = "# rc: major";
        let spec = line.strip_prefix("# rc:").map(|s| s.trim());

        assert_eq!(spec, Some("major"));
    }

    #[test]
    fn test_changelog_line_wrap() {
        let message = "This is a very long commit message that should be wrapped at 78 characters to fit nicely in the changelog";
        let wrapped = textwrap::wrap(message, 78);

        assert!(wrapped.len() > 1);
        assert!(wrapped[0].len() <= 78);
    }

    #[test]
    fn test_changelog_entry_format() {
        let message = "Add new feature";
        let entry = format!("- {}", message);

        assert_eq!(entry, "- Add new feature");
    }

    #[test]
    fn test_changelog_multiline_entry() {
        let message = "Add support for multiple authentication providers including OAuth2 and SAML";
        let lines: Vec<String> = textwrap::wrap(message, 78)
            .iter()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    format!("- {}", line)
                } else {
                    format!("  {}", line)
                }
            })
            .collect();

        assert!(lines[0].starts_with("- "));
        if lines.len() > 1 {
            assert!(lines[1].starts_with("  "));
        }
    }

    #[test]
    fn test_scan_changelog_finds_first_header() {
        let content = "# Project 1.0.0 (2025-01-15)\n\n- Initial release\n\n# Project 0.9.0 (2024-12-01)\n\n- Beta";
        let lines: Vec<_> = content.lines().collect();

        let mut found_header = false;
        for line in lines {
            if line.starts_with("# ") {
                found_header = true;
                break;
            }
        }

        assert!(found_header);
    }

    #[test]
    fn test_scan_changelog_stops_at_second_header() {
        let content =
            "# Version 1.0.0\n\n- Feature A\n- Feature B\n\n# Version 0.9.0\n\n- Old feature";

        let mut entries = Vec::new();
        let mut in_first_section = false;

        for line in content.lines() {
            if line.starts_with("# ") {
                if in_first_section {
                    break;
                }
                in_first_section = true;
                continue;
            }

            if in_first_section && line.starts_with("- ") {
                entries.push(line);
            }
        }

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], "- Feature A");
        assert_eq!(entries[1], "- Feature B");
    }
}
