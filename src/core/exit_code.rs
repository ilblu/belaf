//! Stable exit codes for `belaf`. Documented in the `--help` text and
//! emitted by `belaf describe --json` so AI agents can branch on them
//! without parsing stderr.
//!
//! The set is intentionally small. Add a new variant only when an
//! existing one is genuinely wrong, and never repurpose a number.

/// Stable exit codes. Variants and their numeric values are part of
/// the CLI's public contract — do not change them.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    /// Operation completed successfully.
    Ok = 0,

    /// Catch-all for unexpected internal errors. Equivalent to a panic
    /// at the boundary; agents should retry or escalate.
    Generic = 1,

    /// Caller passed an invalid combination of flags / arguments. The
    /// CLI prints a usage message and exits.
    UsageError = 2,

    /// There is nothing to do. `belaf prepare --ci` in a repo with no
    /// pending commits returns this; `belaf changelog` with no
    /// commits since the last tag also returns this.
    NothingToDo = 3,

    /// A precondition was not met. Most common: dirty working tree
    /// when one is expected to be clean, missing auth, or no GitHub
    /// App installation on the repository.
    Precondition = 4,

    /// A conflict was detected. Examples: a release manifest already
    /// exists at the path that would be written; two `[release_unit]`
    /// blocks claim the same path.
    Conflict = 5,

    /// Network call failed. Talking to `api.belaf.dev` or the GitHub
    /// API raised an HTTP / connection error.
    Network = 6,

    /// `belaf/config.toml` is invalid or could not be parsed.
    ConfigInvalid = 7,
}

impl ExitCode {
    /// Stable string label. Used in `belaf describe --json` so agents
    /// can match on the name rather than the number.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Generic => "generic",
            Self::UsageError => "usage_error",
            Self::NothingToDo => "nothing_to_do",
            Self::Precondition => "precondition",
            Self::Conflict => "conflict",
            Self::Network => "network",
            Self::ConfigInvalid => "config_invalid",
        }
    }

    /// One-line description for `--help` and `describe` output.
    pub fn description(self) -> &'static str {
        match self {
            Self::Ok => "Operation completed successfully.",
            Self::Generic => "Unexpected internal error.",
            Self::UsageError => "Invalid argument or flag combination.",
            Self::NothingToDo => "No work to perform (e.g., no pending commits).",
            Self::Precondition => "Precondition not met (dirty tree, missing auth, etc.).",
            Self::Conflict => "Conflict detected (manifest exists, name collision, etc.).",
            Self::Network => "Network call failed (api.belaf.dev or GitHub API).",
            Self::ConfigInvalid => "`belaf/config.toml` invalid or unparseable.",
        }
    }

    /// Every variant, for documentation generators.
    pub fn all() -> &'static [ExitCode] {
        &[
            Self::Ok,
            Self::Generic,
            Self::UsageError,
            Self::NothingToDo,
            Self::Precondition,
            Self::Conflict,
            Self::Network,
            Self::ConfigInvalid,
        ]
    }
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> i32 {
        code as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_unique() {
        let mut seen: Vec<&'static str> = ExitCode::all().iter().map(|c| c.label()).collect();
        seen.sort_unstable();
        let original_len = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), original_len, "labels must be unique");
    }

    #[test]
    fn numeric_values_are_unique_and_stable() {
        let mut seen: Vec<i32> = ExitCode::all().iter().map(|c| (*c).into()).collect();
        seen.sort_unstable();
        let original_len = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), original_len, "numeric values must be unique");
    }

    #[test]
    fn ok_is_zero() {
        assert_eq!(i32::from(ExitCode::Ok), 0);
    }
}
