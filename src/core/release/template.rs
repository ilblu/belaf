use std::collections::HashMap;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("missing key '{0}' in template")]
    MissingKey(String),
    #[error("unclosed brace in template")]
    UnclosedBrace,
}

pub fn format_template<V: AsRef<str>>(
    template: &str,
    args: &HashMap<&str, V>,
) -> Result<String, TemplateError> {
    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                chars.next();
                result.push('{');
            } else {
                let mut key = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(ch) => key.push(ch),
                        None => return Err(TemplateError::UnclosedBrace),
                    }
                }
                match args.get(key.as_str()) {
                    Some(value) => result.push_str(value.as_ref()),
                    None => return Err(TemplateError::MissingKey(key)),
                }
            }
        } else if c == '}' {
            if chars.peek() == Some(&'}') {
                chars.next();
                result.push('}');
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_replacement() {
        let mut args = HashMap::new();
        args.insert("name", "world");
        let result = format_template("hello, {name}!", &args).unwrap();
        assert_eq!(result, "hello, world!");
    }

    #[test]
    fn test_multiple_replacements() {
        let mut args = HashMap::new();
        args.insert("project_slug", "my-project");
        args.insert("version", "1.0.0");
        args.insert("yyyy_mm_dd", "2025-01-15");
        let result = format_template("# {project_slug} {version} ({yyyy_mm_dd})\n", &args).unwrap();
        assert_eq!(result, "# my-project 1.0.0 (2025-01-15)\n");
    }

    #[test]
    fn test_escaped_braces() {
        let args: HashMap<&str, &str> = HashMap::new();
        let result = format_template("use {{braces}}", &args).unwrap();
        assert_eq!(result, "use {braces}");
    }

    #[test]
    fn test_missing_key() {
        let args: HashMap<&str, &str> = HashMap::new();
        let result = format_template("{missing}", &args);
        assert!(matches!(result, Err(TemplateError::MissingKey(_))));
    }

    #[test]
    fn test_unclosed_brace() {
        let args: HashMap<&str, &str> = HashMap::new();
        let result = format_template("hello {name", &args);
        assert!(matches!(result, Err(TemplateError::UnclosedBrace)));
    }

    #[test]
    fn test_empty_template() {
        let args: HashMap<&str, &str> = HashMap::new();
        let result = format_template("", &args).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_no_placeholders() {
        let args: HashMap<&str, &str> = HashMap::new();
        let result = format_template("plain text", &args).unwrap();
        assert_eq!(result, "plain text");
    }

    #[test]
    fn test_tag_name_format() {
        let mut args = HashMap::new();
        args.insert("project_slug", "belaf");
        args.insert("version", "0.5.0");
        let result = format_template("{project_slug}-v{version}", &args).unwrap();
        assert_eq!(result, "belaf-v0.5.0");
    }
}
