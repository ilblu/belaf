use git2::Commit as GitCommit;
use git2::Signature as CommitSignature;
use git_conventional::Commit as ConventionalCommit;
use git_conventional::Footer as ConventionalFooter;
use lazy_regex::{lazy_regex, Lazy, Regex};
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::Value;

use super::config::{CommitParser, GitConfig, LinkParser, TextProcessor};
use super::contributor::RemoteContributor;
use super::error::{Error, Result};

static SHA1_REGEX: Lazy<Regex> = lazy_regex!(r#"^([a-f0-9]{40}) (.*)$"#);

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Link {
    pub text: String,
    pub href: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct Footer {
    pub token: String,
    pub separator: String,
    pub value: String,
    pub breaking: bool,
}

impl<'a> From<&'a ConventionalFooter<'a>> for Footer {
    fn from(footer: &'a ConventionalFooter<'a>) -> Self {
        Self {
            token: footer.token().as_str().to_owned(),
            separator: footer.separator().as_str().to_owned(),
            value: footer.value().to_owned(),
            breaking: footer.breaking(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ConventionalData {
    pub type_: String,
    pub scope: Option<String>,
    pub description: String,
    pub body: Option<String>,
    pub breaking: bool,
    pub breaking_description: Option<String>,
    pub footers: Vec<Footer>,
}

impl ConventionalData {
    fn from_conventional(conv: &ConventionalCommit<'_>) -> Self {
        Self {
            type_: conv.type_().to_string(),
            scope: conv.scope().map(|s| s.to_string()),
            description: conv.description().to_owned(),
            body: conv.body().map(|s| s.to_owned()),
            breaking: conv.breaking(),
            breaking_description: conv.breaking_description().map(|s| s.to_owned()),
            footers: conv.footers().iter().map(Footer::from).collect(),
        }
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct Signature {
    pub name: Option<String>,
    pub email: Option<String>,
    pub timestamp: i64,
}

impl<'a> From<CommitSignature<'a>> for Signature {
    fn from(signature: CommitSignature<'a>) -> Self {
        Self {
            name: signature.name().map(String::from),
            email: signature.email().map(String::from),
            timestamp: signature.when().seconds(),
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub from: String,
    pub to: String,
}

impl Range {
    pub fn new(from: &Commit, to: &Commit) -> Self {
        Self {
            from: from.id.clone(),
            to: to.id.clone(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Commit {
    pub id: String,
    pub message: String,
    #[serde(skip_deserializing)]
    pub conv: Option<ConventionalData>,
    pub group: Option<String>,
    pub default_scope: Option<String>,
    pub scope: Option<String>,
    pub links: Vec<Link>,
    pub author: Signature,
    pub committer: Signature,
    pub merge_commit: bool,
    pub extra: Option<Value>,
    pub remote: Option<RemoteContributor>,
    pub raw_message: Option<String>,
}

impl From<String> for Commit {
    fn from(message: String) -> Self {
        if let Some(captures) = SHA1_REGEX.captures(&message) {
            if let (Some(id), Some(message)) = (
                captures.get(1).map(|v| v.as_str()),
                captures.get(2).map(|v| v.as_str()),
            ) {
                return Commit {
                    id: id.to_string(),
                    message: message.to_string(),
                    ..Default::default()
                };
            }
        }
        Commit {
            id: String::new(),
            message,
            ..Default::default()
        }
    }
}

impl From<&GitCommit<'_>> for Commit {
    fn from(commit: &GitCommit<'_>) -> Self {
        Commit {
            id: commit.id().to_string(),
            message: commit.message().unwrap_or_default().trim_end().to_string(),
            author: commit.author().into(),
            committer: commit.committer().into(),
            merge_commit: commit.parent_count() > 1,
            ..Default::default()
        }
    }
}

impl Commit {
    pub fn new(id: String, message: String) -> Self {
        Self {
            id,
            message,
            ..Default::default()
        }
    }

    pub fn raw_message(&self) -> &str {
        self.raw_message.as_deref().unwrap_or(&self.message)
    }

    pub fn process(&self, config: &GitConfig) -> Result<Self> {
        let mut commit = self.clone();
        commit = commit.preprocess(&config.commit_preprocessors)?;
        if config.conventional_commits {
            if !config.require_conventional && config.filter_unconventional && !config.split_commits
            {
                commit = commit.into_conventional()?;
            } else if let Ok(conv_commit) = commit.clone().into_conventional() {
                commit = conv_commit;
            }
        }

        commit = commit.parse(
            &config.commit_parsers,
            config.protect_breaking_commits,
            config.filter_commits,
        )?;

        commit = commit.parse_links(&config.link_parsers);

        Ok(commit)
    }

    pub fn into_conventional(mut self) -> Result<Self> {
        let raw = self.raw_message().to_string();
        match ConventionalCommit::parse(&raw) {
            Ok(conv) => {
                self.conv = Some(ConventionalData::from_conventional(&conv));
                Ok(self)
            }
            Err(e) => Err(Error::ParseError(e)),
        }
    }

    pub fn preprocess(mut self, preprocessors: &[TextProcessor]) -> Result<Self> {
        preprocessors.iter().try_for_each(|preprocessor| {
            preprocessor.replace(&mut self.message, vec![("COMMIT_SHA", &self.id)])?;
            Ok::<(), Error>(())
        })?;
        Ok(self)
    }

    fn skip_commit(&self, parser: &CommitParser, protect_breaking: bool) -> bool {
        parser.skip.unwrap_or(false)
            && !(self.conv.as_ref().map(|c| c.breaking).unwrap_or(false) && protect_breaking)
    }

    pub fn parse(
        mut self,
        parsers: &[CommitParser],
        protect_breaking: bool,
        filter: bool,
    ) -> Result<Self> {
        let lookup_context = serde_json::to_value(&self).map_err(|e| {
            Error::FieldError(format!("failed to convert context into value: {e}",))
        })?;
        for parser in parsers {
            let mut regex_checks = Vec::new();
            if let Some(message_regex) = parser.message.as_ref() {
                regex_checks.push((message_regex, self.message.to_string()));
            }
            let body = self.conv.as_ref().and_then(|v| v.body.clone());
            if let Some(body_regex) = parser.body.as_ref() {
                regex_checks.push((body_regex, body.clone().unwrap_or_default()));
            }
            if let (Some(footer_regex), Some(footers)) = (
                parser.footer.as_ref(),
                self.conv.as_ref().map(|v| &v.footers),
            ) {
                regex_checks.extend(footers.iter().map(|f| (footer_regex, f.value.clone())));
            }
            if let (Some(field_name), Some(pattern_regex)) =
                (parser.field.as_ref(), parser.pattern.as_ref())
            {
                let values = if field_name == "body" {
                    vec![body.clone()].into_iter().collect()
                } else {
                    tera::dotted_pointer(&lookup_context, field_name).and_then(|v| match v {
                        Value::String(s) => Some(vec![s.clone()]),
                        Value::Number(_) | Value::Bool(_) | Value::Null => {
                            Some(vec![v.to_string()])
                        }
                        Value::Array(arr) => {
                            let mut values = Vec::new();
                            for item in arr {
                                match item {
                                    Value::String(s) => values.push(s.clone()),
                                    Value::Number(_) | Value::Bool(_) | Value::Null => {
                                        values.push(item.to_string())
                                    }
                                    _ => continue,
                                }
                            }
                            Some(values)
                        }
                        _ => None,
                    })
                };
                match values {
                    Some(values) => {
                        if values.is_empty() {
                            log::trace!("Field '{field_name}' is present but empty");
                        } else {
                            for value in values {
                                regex_checks.push((pattern_regex, value));
                            }
                        }
                    }
                    None => {
                        return Err(Error::FieldError(format!(
                            "field '{field_name}' is missing or has unsupported type (expected a \
                             String, Number, Bool, or Null — or an Array of these scalar values)",
                        )));
                    }
                }
            }
            if parser.sha.clone().map(|v| v.to_lowercase()).as_deref() == Some(&self.id) {
                if self.skip_commit(parser, protect_breaking) {
                    return Err(Error::GroupError(String::from("Skipping commit")));
                } else {
                    self.group = parser.group.clone().or(self.group);
                    self.scope = parser.scope.clone().or(self.scope);
                    self.default_scope = parser.default_scope.clone().or(self.default_scope);
                    return Ok(self);
                }
            }
            for (regex, text) in regex_checks {
                if regex.is_match(text.trim()) {
                    if self.skip_commit(parser, protect_breaking) {
                        return Err(Error::GroupError(String::from("Skipping commit")));
                    } else {
                        let regex_replace = |mut value: String| {
                            for mat in regex.find_iter(&text) {
                                value = regex.replace(mat.as_str(), value).to_string();
                            }
                            value
                        };
                        self.group = parser.group.clone().map(regex_replace);
                        self.scope = parser.scope.clone().map(regex_replace);
                        self.default_scope.clone_from(&parser.default_scope);
                        return Ok(self);
                    }
                }
            }
        }
        if filter {
            Err(Error::GroupError(String::from(
                "Commit does not belong to any group",
            )))
        } else {
            Ok(self)
        }
    }

    pub fn parse_links(mut self, parsers: &[LinkParser]) -> Self {
        for parser in parsers {
            let regex = &parser.pattern;
            let replace = &parser.href;
            for mat in regex.find_iter(&self.message) {
                let m = mat.as_str();
                let text = if let Some(text_replace) = &parser.text {
                    regex.replace(m, text_replace).to_string()
                } else {
                    m.to_string()
                };
                let href = regex.replace(m, replace);
                self.links.push(Link {
                    text,
                    href: href.to_string(),
                });
            }
        }
        self
    }

    fn footers(&self) -> impl Iterator<Item = &Footer> {
        self.conv.iter().flat_map(|conv| conv.footers.iter())
    }
}

impl Serialize for Commit {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        struct SerializeFooters<'a>(&'a Commit);
        impl Serialize for SerializeFooters<'_> {
            fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_seq(self.0.footers())
            }
        }

        let mut commit = serializer.serialize_struct("Commit", 15)?;
        commit.serialize_field("id", &self.id)?;
        if let Some(conv) = &self.conv {
            commit.serialize_field("message", &conv.description)?;
            commit.serialize_field("body", &conv.body)?;
            commit.serialize_field("footers", &SerializeFooters(self))?;
            commit.serialize_field("group", self.group.as_ref().unwrap_or(&conv.type_))?;
            commit.serialize_field("breaking_description", &conv.breaking_description)?;
            commit.serialize_field("breaking", &conv.breaking)?;
            commit.serialize_field(
                "scope",
                &self
                    .scope
                    .as_deref()
                    .or(conv.scope.as_deref())
                    .or(self.default_scope.as_deref()),
            )?;
        } else {
            commit.serialize_field("message", &self.message)?;
            commit.serialize_field("group", &self.group)?;
            commit.serialize_field(
                "scope",
                &self.scope.as_deref().or(self.default_scope.as_deref()),
            )?;
        }

        commit.serialize_field("links", &self.links)?;
        commit.serialize_field("author", &self.author)?;
        commit.serialize_field("committer", &self.committer)?;
        commit.serialize_field("conventional", &self.conv.is_some())?;
        commit.serialize_field("merge_commit", &self.merge_commit)?;
        commit.serialize_field("extra", &self.extra)?;
        if let Some(remote) = &self.remote {
            commit.serialize_field("remote", remote)?;
        }
        commit.serialize_field("raw_message", &self.raw_message())?;
        commit.end()
    }
}

pub(crate) fn commits_to_conventional_commits<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<Commit>, D::Error> {
    let commits = Vec::<Commit>::deserialize(deserializer)?;
    let commits = commits
        .into_iter()
        .map(|commit| commit.clone().into_conventional().unwrap_or(commit))
        .collect();
    Ok(commits)
}
