//! Hand-maintained whitelists for variant-fields whose JSON-Schema definition
//! is intentionally a free-form string (not a closed enum).
//!
//! This lets the wire format remain forward-compatible — a producer can ship
//! a value that consumers don't yet know — while still giving consumers a
//! `Known | Unknown`-style discriminated union at the domain layer.
//!
//! See plan §3 Regel 2.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Ecosystem
// ---------------------------------------------------------------------------

/// All ecosystems belaf knows about today. Adding a new one is one line here
/// plus a `EcosystemRegistry::register(...)` call. **No JSON-Schema change.**
pub const KNOWN_ECOSYSTEMS: &[&str] = &[
    "npm", "cargo", "maven", "pypa", "go", "csproj", "swift", "elixir",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownEcosystem {
    Npm,
    Cargo,
    Maven,
    Pypa,
    Go,
    Csproj,
    Swift,
    Elixir,
}

impl KnownEcosystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Cargo => "cargo",
            Self::Maven => "maven",
            Self::Pypa => "pypa",
            Self::Go => "go",
            Self::Csproj => "csproj",
            Self::Swift => "swift",
            Self::Elixir => "elixir",
        }
    }

    /// Human-facing label for CLI/PR-body output. Must not be used as a
    /// wire-format value — pass `as_str()` for that.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Npm => "Node.js (npm)",
            Self::Cargo => "Rust (Cargo)",
            Self::Maven => "Maven",
            Self::Pypa => "Python (PyPA)",
            Self::Go => "Go",
            Self::Csproj => "C# (.NET)",
            Self::Swift => "Swift",
            Self::Elixir => "Elixir",
        }
    }

    /// Canonical version-bearing file (or glob) the loader scans.
    /// Used by the wizard's "Files to Modify" preview.
    pub fn version_file(&self) -> &'static str {
        match self {
            Self::Npm => "package.json",
            Self::Cargo => "Cargo.toml",
            Self::Maven => "pom.xml",
            Self::Pypa => "pyproject.toml",
            Self::Go => "go.mod",
            Self::Csproj => "*.csproj",
            Self::Swift => "Package.swift",
            Self::Elixir => "mix.exs",
        }
    }

    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "npm" => Some(Self::Npm),
            "cargo" => Some(Self::Cargo),
            "maven" => Some(Self::Maven),
            "pypa" => Some(Self::Pypa),
            "go" => Some(Self::Go),
            "csproj" => Some(Self::Csproj),
            "swift" => Some(Self::Swift),
            "elixir" => Some(Self::Elixir),
            _ => None,
        }
    }
}

/// Discriminated wrapper around the wire-level ecosystem string. Consumers
/// that recognise the ecosystem dispatch via `Known(...)`; consumers that
/// don't render `Unknown(string)` as a fallback (grey badge in the dashboard,
/// raw text in the CLI).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    Known(KnownEcosystem),
    Unknown(String),
}

impl Ecosystem {
    pub fn classify(s: &str) -> Self {
        match KnownEcosystem::from_wire(s) {
            Some(k) => Self::Known(k),
            None => Self::Unknown(s.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Known(k) => k.as_str(),
            Self::Unknown(s) => s,
        }
    }

    /// Human-facing label. For unknown ecosystems we render the raw wire
    /// string — same fallback the dashboard uses (grey badge).
    pub fn display_name(&self) -> &str {
        match self {
            Self::Known(k) => k.display_name(),
            Self::Unknown(s) => s,
        }
    }

    /// Canonical version-bearing file. For unknown ecosystems we have no
    /// way to know — return a placeholder the wizard can render distinctly.
    pub fn version_file(&self) -> &'static str {
        match self {
            Self::Known(k) => k.version_file(),
            Self::Unknown(_) => "(unknown)",
        }
    }
}

impl Serialize for Ecosystem {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Ecosystem {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Self::classify(&s))
    }
}

// ---------------------------------------------------------------------------
// BumpType
// ---------------------------------------------------------------------------

/// Standard bump-type values. Same forward-compat philosophy as ecosystems —
/// a future bump-type (e.g. `release-candidate`) is a one-line addition.
pub const KNOWN_BUMP_TYPES: &[&str] = &["major", "minor", "patch", "prerelease"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownBumpType {
    Major,
    Minor,
    Patch,
    Prerelease,
}

impl KnownBumpType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
            Self::Prerelease => "prerelease",
        }
    }

    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "major" => Some(Self::Major),
            "minor" => Some(Self::Minor),
            "patch" => Some(Self::Patch),
            "prerelease" => Some(Self::Prerelease),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BumpType {
    Known(KnownBumpType),
    Unknown(String),
}

impl BumpType {
    pub fn classify(s: &str) -> Self {
        match KnownBumpType::from_wire(s) {
            Some(k) => Self::Known(k),
            None => Self::Unknown(s.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Known(k) => k.as_str(),
            Self::Unknown(s) => s,
        }
    }
}

impl Serialize for BumpType {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BumpType {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Self::classify(&s))
    }
}

// ---------------------------------------------------------------------------
// ReleaseStatus
// ---------------------------------------------------------------------------

/// Release lifecycle states as tracked in the release-groups DB row. The
/// wire/manifest itself doesn't carry this — it's a downstream-app concept —
/// but we keep the canonical list co-located so producer + consumer agree.
pub const KNOWN_RELEASE_STATUSES: &[&str] = &[
    "pending",
    "in_progress",
    "completed",
    "partial_failure",
    "failed",
    "rolled_back",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownReleaseStatus {
    Pending,
    InProgress,
    Completed,
    PartialFailure,
    Failed,
    RolledBack,
}

impl KnownReleaseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::PartialFailure => "partial_failure",
            Self::Failed => "failed",
            Self::RolledBack => "rolled_back",
        }
    }

    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "partial_failure" => Some(Self::PartialFailure),
            "failed" => Some(Self::Failed),
            "rolled_back" => Some(Self::RolledBack),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReleaseStatus {
    Known(KnownReleaseStatus),
    Unknown(String),
}

impl ReleaseStatus {
    pub fn classify(s: &str) -> Self {
        match KnownReleaseStatus::from_wire(s) {
            Some(k) => Self::Known(k),
            None => Self::Unknown(s.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_classify_known() {
        assert!(matches!(
            Ecosystem::classify("maven"),
            Ecosystem::Known(KnownEcosystem::Maven)
        ));
    }

    #[test]
    fn ecosystem_classify_unknown() {
        match Ecosystem::classify("gradle") {
            Ecosystem::Unknown(s) => assert_eq!(s, "gradle"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn ecosystem_swift_is_known() {
        // Regression: `EcosystemType::from_qname` (the v1.x enum) didn't list
        // swift even though `swift.rs` exists. The 2.0 whitelist must include it.
        assert!(matches!(
            Ecosystem::classify("swift"),
            Ecosystem::Known(KnownEcosystem::Swift)
        ));
    }

    #[test]
    fn known_ecosystems_const_in_sync_with_enum() {
        for s in KNOWN_ECOSYSTEMS {
            assert!(
                KnownEcosystem::from_wire(s).is_some(),
                "KNOWN_ECOSYSTEMS contains {s:?} but KnownEcosystem::from_wire rejects it"
            );
        }
    }

    #[test]
    fn ecosystem_serde_roundtrip() {
        let e = Ecosystem::Known(KnownEcosystem::Npm);
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, "\"npm\"");
        let back: Ecosystem = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn ecosystem_serde_unknown_roundtrip() {
        let e = Ecosystem::Unknown("brand-new-thing".to_string());
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, "\"brand-new-thing\"");
        let back: Ecosystem = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn bump_type_classify() {
        assert!(matches!(
            BumpType::classify("major"),
            BumpType::Known(KnownBumpType::Major)
        ));
        assert!(matches!(BumpType::classify("hotfix"), BumpType::Unknown(_)));
    }

    #[test]
    fn release_status_classify() {
        assert!(matches!(
            ReleaseStatus::classify("rolled_back"),
            ReleaseStatus::Known(KnownReleaseStatus::RolledBack)
        ));
    }
}
