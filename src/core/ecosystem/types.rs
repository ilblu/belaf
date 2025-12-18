use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EcosystemType {
    Cargo,
    Npm,
    Pypa,
    Go,
    Elixir,
    Csproj,
}

impl EcosystemType {
    pub fn from_qname(qname: &str) -> Option<Self> {
        match qname {
            "cargo" => Some(Self::Cargo),
            "npm" => Some(Self::Npm),
            "pypa" => Some(Self::Pypa),
            "go" => Some(Self::Go),
            "elixir" => Some(Self::Elixir),
            "csproj" => Some(Self::Csproj),
            _ => None,
        }
    }

    pub fn version_file(&self) -> &'static str {
        match self {
            Self::Cargo => "Cargo.toml",
            Self::Npm => "package.json",
            Self::Pypa => "pyproject.toml",
            Self::Go => "go.mod",
            Self::Elixir => "mix.exs",
            Self::Csproj => "*.csproj",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Cargo => "Rust (Cargo)",
            Self::Npm => "Node.js (npm)",
            Self::Pypa => "Python (PyPA)",
            Self::Go => "Go",
            Self::Elixir => "Elixir",
            Self::Csproj => "C# (.NET)",
        }
    }
}

impl fmt::Display for EcosystemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}
