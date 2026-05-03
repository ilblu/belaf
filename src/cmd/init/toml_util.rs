//! Tiny TOML emission helpers shared by every place under `cmd::init`
//! that builds config snippets to append to `belaf/config.toml`. The
//! single rule: never let a path / name / version flow into a snippet
//! through raw `format!`. Use [`toml_quote`].

/// Wrap `s` as a TOML basic-string. Properly escapes `"`, `\`, and
/// any control character so a path / name / template containing
/// shell-or-TOML metacharacters can't break out of its slot and
/// inject arbitrary structure into the emitted config.
pub fn toml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r"\\"),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_simple_string() {
        assert_eq!(toml_quote("foo"), "\"foo\"");
    }

    #[test]
    fn escapes_double_quote() {
        assert_eq!(toml_quote(r#"foo"bar"#), r#""foo\"bar""#);
    }

    #[test]
    fn round_trips_via_toml_parser_with_metachars() {
        let nasty = r#"a"b\c]] = inject"#;
        let s = format!("key = {}", toml_quote(nasty));
        let parsed: toml::Value = toml::from_str(&s).expect("must parse as valid TOML");
        assert_eq!(parsed["key"].as_str(), Some(nasty));
    }
}
