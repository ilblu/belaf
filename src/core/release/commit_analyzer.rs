use anyhow::Result;
use git_conventional::{Commit, Type};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpRecommendation {
    Major,
    Minor,
    Patch,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangelogCategory {
    Added,
    Changed,
    Deprecated,
    Removed,
    Fixed,
    Security,
}

impl ChangelogCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Changed => "Changed",
            Self::Deprecated => "Deprecated",
            Self::Removed => "Removed",
            Self::Fixed => "Fixed",
            Self::Security => "Security",
        }
    }

    pub fn from_conventional_type(commit_type: &Type) -> Option<Self> {
        match commit_type.as_str() {
            "feat" => Some(Self::Added),
            "fix" => Some(Self::Fixed),
            "perf" => Some(Self::Changed),
            "refactor" => Some(Self::Changed),
            "docs" | "chore" | "ci" | "test" | "style" | "build" => None,
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CategorizedCommit {
    pub category: ChangelogCategory,
    pub message: String,
    pub scope: Option<String>,
    pub breaking: bool,
    pub original: String,
}

impl CategorizedCommit {
    pub fn format_for_changelog(&self) -> String {
        let scope_part = self
            .scope
            .as_ref()
            .map(|s| format!("**{}**: ", s))
            .unwrap_or_default();
        let breaking_mark = if self.breaking { " [BREAKING]" } else { "" };
        format!("- {}{}{}", scope_part, self.message, breaking_mark)
    }
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

pub fn analyze_commit_messages(messages: &[String]) -> Result<CommitAnalysis> {
    let mut analysis = CommitAnalysis {
        recommendation: BumpRecommendation::None,
        total_commits: messages.len(),
        ..Default::default()
    };

    for message in messages {
        let commit_result = Commit::parse(message);

        let recommendation = match commit_result {
            Ok(commit) => {
                if commit.breaking() {
                    analysis.breaking_count += 1;
                    BumpRecommendation::Major
                } else {
                    let commit_type = commit.type_();
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

pub fn recommend_bump_for_commits(commit_summaries: &[String]) -> Result<BumpRecommendation> {
    let analysis = analyze_commit_messages(commit_summaries)?;
    Ok(analysis.recommendation)
}

pub fn categorize_commits(messages: &[String]) -> Vec<CategorizedCommit> {
    let mut categorized = Vec::new();

    for message in messages {
        let commit_result = Commit::parse(message);

        if let Ok(commit) = commit_result {
            let commit_type = commit.type_();
            let breaking = commit.breaking();

            if breaking {
                categorized.push(CategorizedCommit {
                    category: ChangelogCategory::Changed,
                    message: commit.description().to_string(),
                    scope: commit.scope().map(|s| s.to_string()),
                    breaking: true,
                    original: message.clone(),
                });
            } else if let Some(category) = ChangelogCategory::from_conventional_type(&commit_type) {
                categorized.push(CategorizedCommit {
                    category,
                    message: commit.description().to_string(),
                    scope: commit.scope().map(|s| s.to_string()),
                    breaking: false,
                    original: message.clone(),
                });
            }
        }
    }

    categorized.sort_by_key(|c| c.category);
    categorized
}

pub fn extract_scope(message: &str) -> Option<String> {
    Commit::parse(message)
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
}
