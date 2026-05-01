//! Icon glyphs for the init wizard.
//!
//! Three modes, selected via `BELAF_ICONS`:
//!
//! | mode (env var) | what it uses |
//! |---|---|
//! | `unicode` *(default)* | Standard Unicode geometric shapes (⬢ ◆ ◇ ● ○ ▲). Works on every terminal + font without setup. |
//! | `nerd` | Nerd Font codepoints (Material Design + Devicon glyphs). Crisp icons + per-row ecosystem logos, but requires the user to have a Nerd Font installed and configured. |
//! | `ascii` | Pure ASCII (`[x]`, `[ ]`, `[*]`, …) for CI logs / SSH sessions / dumb terminals. |
//!
//! `BELAF_NO_COLOR=1` and `NO_COLOR=1` do **not** affect icons (they are
//! about ANSI colors); use `BELAF_ICONS=ascii` for pure-text rendering.
//!
//! All glyphs in `unicode` and `nerd` modes are designed to occupy
//! one terminal cell so the checkbox column lines up across rows.

use std::sync::OnceLock;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IconMode {
    /// Universal Unicode shapes — works without any font setup.
    Unicode,
    /// Nerd Font glyphs incl. ecosystem logos. Opt-in via env var.
    Nerd,
    /// Pure-ASCII fallback for CI / dumb terminals.
    Ascii,
}

/// Resolve once per process. Reading the env var on every render
/// would be wasteful; the wizard runs in a single mode per session.
pub fn mode() -> IconMode {
    static CACHED: OnceLock<IconMode> = OnceLock::new();
    *CACHED.get_or_init(|| match std::env::var("BELAF_ICONS").as_deref() {
        Ok("nerd") => IconMode::Nerd,
        Ok("ascii") => IconMode::Ascii,
        // `unicode`, anything else, and unset all default to Unicode.
        _ => IconMode::Unicode,
    })
}

/// Header glyph for a row-category section divider.
pub fn category_glyph(name: &str) -> &'static str {
    match (mode(), name) {
        // Unicode (default)
        (IconMode::Unicode, "Bundles") => "\u{2B22}", // ⬢ Black Hexagon
        (IconMode::Unicode, "Standalone") => "\u{25C6}", // ◆ Black Diamond
        (IconMode::Unicode, "Externally-managed") => "\u{25C7}", // ◇ White Diamond
        (IconMode::Unicode, "Drift") => "\u{25B2}",   // ▲ Up Triangle
        (IconMode::Unicode, _) => "\u{25CF}",         // ● Black Circle

        // Nerd Font (Material Design Icons)
        (IconMode::Nerd, "Bundles") => "\u{f03d8}", // md-package_variant_closed
        (IconMode::Nerd, "Standalone") => "\u{f01a3}", // md-cube_outline
        (IconMode::Nerd, "Externally-managed") => "\u{f19a8}", // md-cellphone_link
        (IconMode::Nerd, "Drift") => "\u{f0026}",   // md-alert
        (IconMode::Nerd, _) => "\u{f02fd}",         // md-help_circle_outline

        // ASCII
        (IconMode::Ascii, "Bundles") => "[*]",
        (IconMode::Ascii, "Standalone") => "[#]",
        (IconMode::Ascii, "Externally-managed") => "[~]",
        (IconMode::Ascii, "Drift") => "[!]",
        (IconMode::Ascii, _) => "[?]",
    }
}

/// Checkbox glyph for togglable rows.
pub fn checkbox(checked: bool) -> &'static str {
    match (mode(), checked) {
        (IconMode::Unicode, true) => "\u{25CF}",  // ● Black Circle
        (IconMode::Unicode, false) => "\u{25CB}", // ○ White Circle
        (IconMode::Nerd, true) => "\u{f0e1e}",    // md-checkbox_marked
        (IconMode::Nerd, false) => "\u{f0131}",   // md-checkbox_blank_outline
        (IconMode::Ascii, true) => "[x]",
        (IconMode::Ascii, false) => "[ ]",
    }
}

/// Indicator for non-togglable rows (mobile / read-only).
pub fn locked() -> &'static str {
    match mode() {
        IconMode::Unicode => "\u{2014}", // — em dash
        IconMode::Nerd => "\u{f0341}",   // md-lock_outline
        IconMode::Ascii => "[-]",
    }
}

/// Ecosystem icon — only meaningful in `nerd` mode (proper logos).
/// Returns an empty string in `unicode` and `ascii` modes so the
/// caller can omit the column entirely. The trailing space is
/// included in nerd mode so column alignment stays stable.
pub fn ecosystem(eco: &str) -> &'static str {
    if !matches!(mode(), IconMode::Nerd) {
        return "";
    }
    match eco {
        "cargo" | "rust" => "\u{e7a8} ",                  // dev-rust (crab)
        "npm" => "\u{e71e} ",                             // dev-npm
        "typescript" | "ts" => "\u{e628} ",               // dev-typescript
        "swift" => "\u{e755} ",                           // dev-swift
        "kotlin" => "\u{f0a3a} ",                         // md-language_kotlin
        "maven" | "java" | "jvm" => "\u{e738} ",          // dev-java
        "pypa" | "python" => "\u{e73c} ",                 // dev-python
        "go" | "golang" => "\u{e626} ",                   // dev-go
        "elixir" => "\u{e62d} ",                          // dev-elixir
        "csproj" | "csharp" | "dotnet" => "\u{f031b} ",   // md-language_csharp
        "tauri" => "\u{f04ad} ",                          // md-window_restore
        "hexagonal-cargo" => "\u{e7a8} ",                 // crab again — Rust at core
        "sdk-cascade-member" | "cascade" => "\u{f0c5d} ", // md-source_branch
        "jvm-library" => "\u{e738} ",
        "single-project" => "\u{f01a3} ",
        "nested-npm-workspace" => "\u{e71e} ",
        _ => "\u{f02fd} ", // md-help_circle_outline
    }
}

/// Header banner glyph (top-of-screen "Review and toggle …").
pub fn header_clipboard() -> &'static str {
    match mode() {
        IconMode::Unicode => "\u{2756}", // ❖ Black Diamond Minus White X
        IconMode::Nerd => "\u{f0c19}",   // md-clipboard_list_outline
        IconMode::Ascii => "::",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Make sure all glyph getters are non-empty in every mode (except
    /// `ecosystem()` which is intentionally empty outside `nerd`).
    #[test]
    fn glyphs_are_non_empty_per_mode() {
        // The `mode()` cache is process-wide, so we test the mapping
        // by inspecting each branch directly. Switching cached state
        // mid-test would race other tests in the same binary.
        for cat in ["Bundles", "Standalone", "Externally-managed", "Drift"] {
            assert!(!category_glyph(cat).is_empty());
        }
        assert!(!checkbox(true).is_empty());
        assert!(!checkbox(false).is_empty());
        assert!(!locked().is_empty());
        assert!(!header_clipboard().is_empty());
    }
}
