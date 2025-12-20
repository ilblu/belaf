use anyhow::Result;
use git_conventional::Type;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::core::changelog::Commit;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BumpConfig {
    pub features_always_bump_minor: bool,

    pub breaking_always_bump_major: bool,

    pub initial_tag: String,

    pub bump_type: Option<String>,
}

impl BumpConfig {
    pub fn from_user_config(cfg: &super::config::syntax::BumpConfiguration) -> Self {
        Self {
            features_always_bump_minor: cfg.features_always_bump_minor,
            breaking_always_bump_major: cfg.breaking_always_bump_major,
            initial_tag: cfg.initial_tag.clone(),
            bump_type: cfg.bump_type.clone(),
        }
    }
}

impl From<&super::config::syntax::BumpConfiguration> for BumpConfig {
    fn from(cfg: &super::config::syntax::BumpConfiguration) -> Self {
        Self::from_user_config(cfg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpRecommendation {
    Major,
    Minor,
    Patch,
    None,
}

impl BumpRecommendation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Patch => "patch",
            Self::None => "no bump",
        }
    }

    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Major, _) | (_, Self::Major) => Self::Major,
            (Self::Minor, _) | (_, Self::Minor) => Self::Minor,
            (Self::Patch, _) | (_, Self::Patch) => Self::Patch,
            (Self::None, Self::None) => Self::None,
        }
    }

    pub fn apply_config(self, config: &BumpConfig, current_version: Option<&str>) -> Self {
        let is_pre_1_0 = current_version
            .and_then(|v| {
                let v = v.trim_start_matches('v');
                v.split('.').next()?.parse::<u32>().ok()
            })
            .map(|major| major == 0)
            .unwrap_or(false);

        if is_pre_1_0 {
            match self {
                Self::Major if !config.breaking_always_bump_major => Self::Minor,
                Self::Minor if !config.features_always_bump_minor => Self::Patch,
                other => other,
            }
        } else {
            self
        }
    }

    pub fn from_string(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "major" => Some(Self::Major),
            "minor" => Some(Self::Minor),
            "patch" => Some(Self::Patch),
            _ => None,
        }
    }

    pub fn from_commits_with_config(
        commits: &[String],
        config: &BumpConfig,
        current_version: Option<&str>,
    ) -> Result<Self> {
        let analysis = analyze_commit_messages(commits)?;
        Ok(analysis.recommendation.apply_config(config, current_version))
    }

    pub fn from_config_or_commits(
        commits: &[String],
        config: &BumpConfig,
        current_version: Option<&str>,
    ) -> Result<(Self, bool)> {
        if let Some(bump_str) = &config.bump_type {
            if let Some(bump) = Self::from_string(bump_str) {
                return Ok((bump, true));
            }
        }
        let recommendation = Self::from_commits_with_config(commits, config, current_version)?;
        Ok((recommendation, false))
    }
}

#[derive(Debug)]
pub struct CommitAnalysis {
    pub recommendation: BumpRecommendation,
    pub total_commits: usize,
    pub feat_count: usize,
    pub fix_count: usize,
    pub breaking_count: usize,
    pub other_count: usize,
}

impl Default for CommitAnalysis {
    fn default() -> Self {
        Self {
            recommendation: BumpRecommendation::None,
            total_commits: 0,
            feat_count: 0,
            fix_count: 0,
            breaking_count: 0,
            other_count: 0,
        }
    }
}

impl CommitAnalysis {
    pub fn summary(&self) -> String {
        if self.total_commits == 0 {
            return "No commits to analyze".to_string();
        }

        let mut parts = Vec::new();

        if self.feat_count > 0 {
            parts.push(format!("{} feat", self.feat_count));
        }
        if self.fix_count > 0 {
            parts.push(format!("{} fix", self.fix_count));
        }
        if self.breaking_count > 0 {
            parts.push(format!("{} BREAKING", self.breaking_count));
        }
        if self.other_count > 0 {
            parts.push(format!("{} other", self.other_count));
        }

        let commits_text = parts.join(", ");

        format!(
            "{} commit{}: {} → suggests {}",
            self.total_commits,
            if self.total_commits == 1 { "" } else { "s" },
            commits_text,
            match self.recommendation {
                BumpRecommendation::Major => "MAJOR",
                BumpRecommendation::Minor => "MINOR",
                BumpRecommendation::Patch => "PATCH",
                BumpRecommendation::None => "NO BUMP",
            }
        )
    }
}

pub fn analyze_commits(commits: &[Commit<'static>]) -> Result<CommitAnalysis> {
    let mut analysis = CommitAnalysis {
        recommendation: BumpRecommendation::None,
        total_commits: commits.len(),
        ..Default::default()
    };

    for commit in commits {
        let commit_result = git_conventional::Commit::parse(&commit.message);

        let recommendation = match commit_result {
            Ok(conv) => {
                if conv.breaking() {
                    analysis.breaking_count += 1;
                    BumpRecommendation::Major
                } else {
                    let commit_type = conv.type_();
                    if commit_type == Type::FEAT {
                        analysis.feat_count += 1;
                        BumpRecommendation::Minor
                    } else if commit_type == Type::FIX || commit_type == Type::PERF {
                        analysis.fix_count += 1;
                        BumpRecommendation::Patch
                    } else {
                        analysis.other_count += 1;
                        BumpRecommendation::None
                    }
                }
            }
            Err(_) => {
                analysis.other_count += 1;
                BumpRecommendation::None
            }
        };

        analysis.recommendation = analysis.recommendation.merge(recommendation);
    }

    Ok(analysis)
}

pub fn analyze_commit_messages(messages: &[String]) -> Result<CommitAnalysis> {
    let commits: Vec<Commit<'static>> = messages
        .iter()
        .map(|msg| Commit {
            message: msg.clone(),
            ..Default::default()
        })
        .collect();
    analyze_commits(&commits)
}

pub fn recommend_bump_for_commits(commit_summaries: &[String]) -> Result<BumpRecommendation> {
    let analysis = analyze_commit_messages(commit_summaries)?;
    Ok(analysis.recommendation)
}

pub fn extract_scope(message: &str) -> Option<String> {
    git_conventional::Commit::parse(message)
        .ok()
        .and_then(|c| c.scope().map(|s| s.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeMatchMode {
    #[default]
    Smart,
    Exact,
    Suffix,
    Contains,
}

impl std::str::FromStr for ScopeMatchMode {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "exact" => Self::Exact,
            "suffix" => Self::Suffix,
            "contains" => Self::Contains,
            _ => Self::Smart,
        })
    }
}

pub struct ScopeMatcher {
    mode: ScopeMatchMode,
    scope_mappings: HashMap<String, String>,
    package_scopes: HashMap<String, Vec<String>>,
}

impl Default for ScopeMatcher {
    fn default() -> Self {
        Self {
            mode: ScopeMatchMode::Smart,
            scope_mappings: HashMap::new(),
            package_scopes: HashMap::new(),
        }
    }
}

impl ScopeMatcher {
    pub fn new(
        mode: ScopeMatchMode,
        scope_mappings: HashMap<String, String>,
        package_scopes: HashMap<String, Vec<String>>,
    ) -> Self {
        Self {
            mode,
            scope_mappings,
            package_scopes,
        }
    }

    pub fn find_matching_project<'a>(
        &self,
        scope: &str,
        project_names: &'a [String],
    ) -> Option<&'a String> {
        let scope_lower = scope.to_lowercase();

        if let Some(mapped) = self.scope_mappings.get(&scope_lower) {
            return project_names
                .iter()
                .find(|p| p.to_lowercase() == mapped.to_lowercase());
        }

        for (pkg, scopes) in &self.package_scopes {
            if scopes.iter().any(|s| s.to_lowercase() == scope_lower) {
                return project_names
                    .iter()
                    .find(|p| p.to_lowercase() == pkg.to_lowercase());
            }
        }

        match self.mode {
            ScopeMatchMode::Exact => self.match_exact(&scope_lower, project_names),
            ScopeMatchMode::Suffix => self.match_suffix(&scope_lower, project_names),
            ScopeMatchMode::Contains => self.match_contains(&scope_lower, project_names),
            ScopeMatchMode::Smart => self.match_smart(&scope_lower, project_names),
        }
    }

    fn match_exact<'a>(&self, scope: &str, project_names: &'a [String]) -> Option<&'a String> {
        project_names.iter().find(|p| p.to_lowercase() == scope)
    }

    fn match_suffix<'a>(&self, scope: &str, project_names: &'a [String]) -> Option<&'a String> {
        project_names.iter().find(|p| {
            let p_lower = p.to_lowercase();
            p_lower == scope
                || p_lower.ends_with(&format!("-{}", scope))
                || p_lower.ends_with(&format!("_{}", scope))
        })
    }

    fn match_contains<'a>(&self, scope: &str, project_names: &'a [String]) -> Option<&'a String> {
        project_names
            .iter()
            .find(|p| p.to_lowercase().contains(scope))
    }

    fn match_smart<'a>(&self, scope: &str, project_names: &'a [String]) -> Option<&'a String> {
        if let Some(found) = self.match_exact(scope, project_names) {
            return Some(found);
        }
        if let Some(found) = self.match_suffix(scope, project_names) {
            return Some(found);
        }
        self.match_contains(scope, project_names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feat_recommends_minor() {
        let commits = vec!["feat: add new feature".to_string()];
        let recommendation = recommend_bump_for_commits(&commits).unwrap();
        assert_eq!(recommendation, BumpRecommendation::Minor);
    }

    #[test]
    fn test_fix_recommends_patch() {
        let commits = vec!["fix: correct bug".to_string()];
        let recommendation = recommend_bump_for_commits(&commits).unwrap();
        assert_eq!(recommendation, BumpRecommendation::Patch);
    }

    #[test]
    fn test_breaking_recommends_major() {
        let commits = vec!["feat!: breaking change".to_string()];
        let recommendation = recommend_bump_for_commits(&commits).unwrap();
        assert_eq!(recommendation, BumpRecommendation::Major);
    }

    #[test]
    fn test_breaking_footer_recommends_major() {
        let commits = vec!["feat: add feature\n\nBREAKING CHANGE: breaks API".to_string()];
        let recommendation = recommend_bump_for_commits(&commits).unwrap();
        assert_eq!(recommendation, BumpRecommendation::Major);
    }

    #[test]
    fn test_mixed_commits_picks_highest() {
        let commits = vec![
            "fix: bug fix".to_string(),
            "feat: new feature".to_string(),
            "docs: update README".to_string(),
        ];
        let recommendation = recommend_bump_for_commits(&commits).unwrap();
        assert_eq!(recommendation, BumpRecommendation::Minor);
    }

    #[test]
    fn test_analysis_summary() {
        let commits = vec![
            "feat: add feature".to_string(),
            "fix: fix bug".to_string(),
            "docs: update".to_string(),
        ];
        let analysis = analyze_commit_messages(&commits).unwrap();
        assert_eq!(analysis.feat_count, 1);
        assert_eq!(analysis.fix_count, 1);
        assert_eq!(analysis.other_count, 1);
        assert_eq!(analysis.recommendation, BumpRecommendation::Minor);
    }

    #[test]
    fn test_extract_scope() {
        assert_eq!(
            extract_scope("feat(auth): add login"),
            Some("auth".to_string())
        );
        assert_eq!(
            extract_scope("fix(gate): resolve bug"),
            Some("gate".to_string())
        );
        assert_eq!(extract_scope("feat: no scope"), None);
        assert_eq!(extract_scope("random message"), None);
    }

    #[test]
    fn test_scope_matcher_exact() {
        let matcher = ScopeMatcher::new(ScopeMatchMode::Exact, HashMap::new(), HashMap::new());
        let projects = vec![
            "gate".to_string(),
            "jiji".to_string(),
            "belaf-jwt".to_string(),
        ];

        assert_eq!(
            matcher.find_matching_project("gate", &projects),
            Some(&"gate".to_string())
        );
        assert_eq!(matcher.find_matching_project("jwt", &projects), None);
    }

    #[test]
    fn test_scope_matcher_suffix() {
        let matcher = ScopeMatcher::new(ScopeMatchMode::Suffix, HashMap::new(), HashMap::new());
        let projects = vec![
            "gate".to_string(),
            "belaf-jwt".to_string(),
            "belaf-events".to_string(),
        ];

        assert_eq!(
            matcher.find_matching_project("jwt", &projects),
            Some(&"belaf-jwt".to_string())
        );
        assert_eq!(
            matcher.find_matching_project("events", &projects),
            Some(&"belaf-events".to_string())
        );
        assert_eq!(
            matcher.find_matching_project("gate", &projects),
            Some(&"gate".to_string())
        );
    }

    #[test]
    fn test_scope_matcher_smart() {
        let matcher = ScopeMatcher::default();
        let projects = vec![
            "gate".to_string(),
            "jiji".to_string(),
            "belaf-jwt".to_string(),
        ];

        assert_eq!(
            matcher.find_matching_project("gate", &projects),
            Some(&"gate".to_string())
        );
        assert_eq!(
            matcher.find_matching_project("jwt", &projects),
            Some(&"belaf-jwt".to_string())
        );
        assert_eq!(matcher.find_matching_project("unknown", &projects), None);
    }

    #[test]
    fn test_scope_matcher_with_mappings() {
        let mut mappings = HashMap::new();
        mappings.insert("auth".to_string(), "belaf-jwt".to_string());

        let matcher = ScopeMatcher::new(ScopeMatchMode::Exact, mappings, HashMap::new());
        let projects = vec!["gate".to_string(), "belaf-jwt".to_string()];

        assert_eq!(
            matcher.find_matching_project("auth", &projects),
            Some(&"belaf-jwt".to_string())
        );
    }

    #[test]
    fn test_scope_matcher_with_package_scopes() {
        let mut package_scopes = HashMap::new();
        package_scopes.insert(
            "belaf-jwt".to_string(),
            vec!["jwt".to_string(), "token".to_string(), "auth".to_string()],
        );

        let matcher = ScopeMatcher::new(ScopeMatchMode::Exact, HashMap::new(), package_scopes);
        let projects = vec!["gate".to_string(), "belaf-jwt".to_string()];

        assert_eq!(
            matcher.find_matching_project("token", &projects),
            Some(&"belaf-jwt".to_string())
        );
        assert_eq!(
            matcher.find_matching_project("auth", &projects),
            Some(&"belaf-jwt".to_string())
        );
    }

    #[test]
    fn test_apply_config_pre_1_0_default() {
        let config = BumpConfig {
            features_always_bump_minor: true,
            breaking_always_bump_major: true,
            initial_tag: "0.1.0".to_string(),
            bump_type: None,
        };
        assert_eq!(
            BumpRecommendation::Major.apply_config(&config, Some("0.5.0")),
            BumpRecommendation::Major
        );
        assert_eq!(
            BumpRecommendation::Minor.apply_config(&config, Some("0.5.0")),
            BumpRecommendation::Minor
        );
    }

    #[test]
    fn test_apply_config_pre_1_0_conservative() {
        let config = BumpConfig {
            features_always_bump_minor: false,
            breaking_always_bump_major: false,
            initial_tag: "0.1.0".to_string(),
            bump_type: None,
        };
        assert_eq!(
            BumpRecommendation::Major.apply_config(&config, Some("0.5.0")),
            BumpRecommendation::Minor
        );
        assert_eq!(
            BumpRecommendation::Minor.apply_config(&config, Some("0.5.0")),
            BumpRecommendation::Patch
        );
    }

    #[test]
    fn test_apply_config_post_1_0_ignores_config() {
        let config = BumpConfig {
            features_always_bump_minor: false,
            breaking_always_bump_major: false,
            initial_tag: "0.1.0".to_string(),
            bump_type: None,
        };
        assert_eq!(
            BumpRecommendation::Major.apply_config(&config, Some("1.0.0")),
            BumpRecommendation::Major
        );
        assert_eq!(
            BumpRecommendation::Minor.apply_config(&config, Some("2.3.4")),
            BumpRecommendation::Minor
        );
    }

    #[test]
    fn test_apply_config_with_v_prefix() {
        let config = BumpConfig {
            features_always_bump_minor: false,
            breaking_always_bump_major: false,
            initial_tag: "0.1.0".to_string(),
            bump_type: None,
        };
        assert_eq!(
            BumpRecommendation::Minor.apply_config(&config, Some("v0.5.0")),
            BumpRecommendation::Patch
        );
        assert_eq!(
            BumpRecommendation::Minor.apply_config(&config, Some("v1.0.0")),
            BumpRecommendation::Minor
        );
    }
}
