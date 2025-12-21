mod common;
use common::TestRepo;

fn setup_basic_cargo_project(repo: &TestRepo) {
    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-crate"
version = "0.5.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");
}

fn setup_post_1_0_cargo_project(repo: &TestRepo) {
    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "test-crate"
version = "2.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");
}

fn write_custom_config(repo: &TestRepo, config_content: &str) {
    repo.write_file("belaf/config.toml", config_content);
    repo.commit("chore: update config");
}

fn base_config(bump_features_minor: bool, bump_breaking_major: bool) -> String {
    format!(
        r#"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = """
# Changelog
"""
body = "{{{{ version }}}}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = true
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = {bump_features_minor}
breaking_always_bump_major = {bump_breaking_major}
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"#
    )
}

#[test]
fn test_bump_features_always_bump_minor_true() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    write_custom_config(&repo, &base_config(true, true));

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("0.6.0") || stdout.contains("minor"),
        "Expected minor bump (0.5.0 -> 0.6.0) with features_always_bump_minor=true, got: {stdout}"
    );
}

#[test]
fn test_bump_features_always_bump_minor_false() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    write_custom_config(&repo, &base_config(false, false));

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("0.5.1") || stdout.contains("patch"),
        "Expected patch bump (0.5.0 -> 0.5.1) with features_always_bump_minor=false for pre-1.0, got: {stdout}"
    );
}

#[test]
fn test_bump_breaking_always_bump_major_true() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    write_custom_config(&repo, &base_config(true, true));

    repo.write_file("src/breaking.rs", "pub fn breaking() {}");
    repo.commit("feat!: breaking change");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("1.0.0") || stdout.contains("major"),
        "Expected major bump (0.5.0 -> 1.0.0) with breaking_always_bump_major=true, got: {stdout}"
    );
}

#[test]
fn test_bump_breaking_always_bump_major_false() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    write_custom_config(&repo, &base_config(false, false));

    repo.write_file("src/breaking.rs", "pub fn breaking() {}");
    repo.commit("feat!: breaking change");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("0.6.0") || stdout.contains("minor"),
        "Expected minor bump (0.5.0 -> 0.6.0) with breaking_always_bump_major=false for pre-1.0, got: {stdout}"
    );
}

#[test]
fn test_bump_config_ignored_post_1_0() {
    let repo = TestRepo::new();
    setup_post_1_0_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    write_custom_config(&repo, &base_config(false, false));

    repo.write_file("src/breaking.rs", "pub fn breaking() {}");
    repo.commit("feat!: breaking change");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("3.0.0") || stdout.contains("major"),
        "Expected major bump (2.0.0 -> 3.0.0) for post-1.0 breaking change (config ignored), got: {stdout}"
    );
}

#[test]
fn test_commit_attribution_strategy_scope_first() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/api\", \"crates/web\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "crates/api/Cargo.toml",
        r#"[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/api/src/lib.rs", "pub fn api() {}\n");
    repo.write_file(
        "crates/web/Cargo.toml",
        r#"[package]
name = "web"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/web/src/lib.rs", "pub fn web() {}\n");
    repo.commit("Initial commit");

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = true
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("crates/web/src/feature.rs", "pub fn feature() {}");
    repo.commit("feat(api): add feature with api scope but web file change");

    let output = repo.run_belaf_command(&["status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("api"),
        "With scope_first strategy, commit should be attributed to 'api' based on scope, got: {stdout}"
    );
}

#[test]
fn test_commit_attribution_scope_matching_exact() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/belaf-api\", \"crates/belaf-web\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "crates/belaf-api/Cargo.toml",
        r#"[package]
name = "belaf-api"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/belaf-api/src/lib.rs", "pub fn api() {}\n");
    repo.write_file(
        "crates/belaf-web/Cargo.toml",
        r#"[package]
name = "belaf-web"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/belaf-web/src/lib.rs", "pub fn web() {}\n");
    repo.commit("Initial commit");

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = true
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "exact"
"##;
    write_custom_config(&repo, config);

    repo.write_file("crates/belaf-api/src/feature.rs", "pub fn feature() {}");
    repo.commit("feat(api): add feature with partial scope match");

    let output = repo.run_belaf_command(&["status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !stdout.contains("belaf-api") || stdout.contains("api"),
        "With exact matching, 'api' scope should not match 'belaf-api', got: {stdout}"
    );
}

#[test]
fn test_commit_attribution_scope_mappings() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/belaf-jwt\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "crates/belaf-jwt/Cargo.toml",
        r#"[package]
name = "belaf-jwt"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/belaf-jwt/src/lib.rs", "pub fn jwt() {}\n");
    repo.commit("Initial commit");

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = true
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "exact"

[commit_attribution.scope_mappings]
auth = "belaf-jwt"
token = "belaf-jwt"
"##;
    write_custom_config(&repo, config);

    repo.write_file("crates/belaf-jwt/src/feature.rs", "pub fn feature() {}");
    repo.commit("feat(auth): add authentication feature");

    let output = repo.run_belaf_command(&["status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("belaf-jwt") || stdout.contains("jwt"),
        "With scope_mappings, 'auth' scope should map to 'belaf-jwt', got: {stdout}"
    );
}

#[test]
fn test_commit_attribution_package_scopes() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/belaf-jwt\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "crates/belaf-jwt/Cargo.toml",
        r#"[package]
name = "belaf-jwt"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/belaf-jwt/src/lib.rs", "pub fn jwt() {}\n");
    repo.commit("Initial commit");

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = true
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "exact"

[commit_attribution.package_scopes]
belaf-jwt = ["jwt", "token", "auth", "security"]
"##;
    write_custom_config(&repo, config);

    repo.write_file("crates/belaf-jwt/src/feature.rs", "pub fn feature() {}");
    repo.commit("feat(security): add security feature");

    let output = repo.run_belaf_command(&["status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("belaf-jwt") || stdout.contains("jwt"),
        "With package_scopes, 'security' scope should match 'belaf-jwt', got: {stdout}"
    );
}

#[test]
fn test_changelog_custom_header() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = """
# My Custom Changelog Header

This is a custom header.
"""
body = """
## {{ version }}
{% for commit in commits %}
- {{ commit.message }}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("add new feature"),
        "Changelog should contain feature commit, got: {stdout}"
    );
}

#[test]
fn test_changelog_custom_output_path() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for commit in commits %}
- {{ commit.message }}
{% endfor %}
"""
trim = true
output = "docs/HISTORY.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["changelog"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        repo.file_exists("docs/HISTORY.md"),
        "Changelog should be created at custom path docs/HISTORY.md"
    );
}

#[test]
fn test_changelog_filter_unconventional_true() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for commit in commits %}
- {{ commit.message }}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = true
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    repo.write_file("src/random.rs", "pub fn random() {}");
    repo.commit("random: This is not a conventional commit message");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("add new feature"),
        "Changelog should contain conventional commit"
    );
}

#[test]
fn test_changelog_sort_commits_newest() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for commit in commits %}
- {{ commit.message }}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "newest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/first.rs", "pub fn first() {}");
    repo.commit("feat: first feature");

    repo.write_file("src/second.rs", "pub fn second() {}");
    repo.commit("feat: second feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_pos = stdout.find("first feature");
    let second_pos = stdout.find("second feature");

    assert!(
        first_pos.is_some() && second_pos.is_some(),
        "Both commits should be in changelog"
    );
    assert!(
        second_pos.unwrap() < first_pos.unwrap(),
        "With sort_commits=newest, second feature should appear before first feature"
    );
}

#[test]
fn test_changelog_include_breaking_section() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% if include_breaking_section %}
{% set breaking_commits = commits | filter(attribute="breaking", value=true) %}
{% if breaking_commits | length > 0 %}
### Breaking Changes
{% for commit in breaking_commits %}
- {{ commit.message }}
{% endfor %}
{% endif %}
{% endif %}
{% for group, group_commits in commits | group_by(attribute="group") %}
### {{ group }}
{% for commit in group_commits %}
- {{ commit.message }}
{% endfor %}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = true
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/breaking.rs", "pub fn breaking() {}");
    repo.commit("feat!: breaking API change");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Breaking Changes") || stdout.contains("breaking"),
        "Changelog should contain breaking changes section when include_breaking_section=true, got: {stdout}"
    );
}

#[test]
fn test_changelog_include_statistics() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for group, group_commits in commits | group_by(attribute="group") %}
### {{ group }}
{% for commit in group_commits %}
- {{ commit.message }}
{% endfor %}
{% endfor %}
{% if include_statistics %}
### Statistics
- Total commits: {{ commits | length }}
{% endif %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = true
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feat1.rs", "pub fn feat1() {}");
    repo.commit("feat: first feature");

    repo.write_file("src/feat2.rs", "pub fn feat2() {}");
    repo.commit("feat: second feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Statistics") || stdout.contains("Total commits"),
        "Changelog should contain statistics when include_statistics=true, got: {stdout}"
    );
}

#[test]
fn test_changelog_include_contributors() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for group, group_commits in commits | group_by(attribute="group") %}
### {{ group }}
{% for commit in group_commits %}
- {{ commit.message }}{% if commit.author.name %} by {{ commit.author.name }}{% endif %}
{% endfor %}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = true
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feat1.rs", "pub fn feat1() {}");
    repo.commit("feat: first feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(" by "),
        "Changelog should contain author names when commit.author.name is available, got: {stdout}"
    );
}

#[test]
fn test_changelog_emoji_groups() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for group, group_commits in commits | group_by(attribute="group") %}
### {% if emoji_groups and group_emojis[group] %}{{ group_emojis[group] }} {% endif %}{{ group }}
{% for commit in group_commits %}
- {{ commit.message }}
{% endfor %}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = true

[changelog.group_emojis]
Features = "✨"
"Bug Fixes" = "🐛"

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add sparkle feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("✨") || stdout.contains("Features"),
        "Changelog should contain emoji when emoji_groups=true, got: {stdout}"
    );
}

#[test]
fn test_changelog_custom_commit_parsers() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for group, group_commits in commits | group_by(attribute="group") %}
### {{ group }}
{% for commit in group_commits %}
- {{ commit.message }}
{% endfor %}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^awesome"
group = "Awesome Stuff"

[[changelog.commit_parsers]]
message = "^feat"
group = "New Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bugfixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("New Features"),
        "Changelog should use custom group name 'New Features', got: {stdout}"
    );
}

#[test]
fn test_changelog_commit_parser_skip() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for group, group_commits in commits | group_by(attribute="group") %}
### {{ group }}
{% for commit in group_commits %}
- {{ commit.message }}
{% endfor %}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = true
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[[changelog.commit_parsers]]
message = "^chore\\(deps\\)"
skip = true

[[changelog.commit_parsers]]
message = "^chore\\(release\\)"
skip = true

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    repo.write_file("src/deps.rs", "pub fn deps() {}");
    repo.commit("chore(deps): update dependencies");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("add new feature"),
        "Changelog should contain feature commit"
    );
    assert!(
        !stdout.contains("update dependencies"),
        "Changelog should NOT contain skipped deps commit"
    );
}

#[test]
fn test_changelog_postprocessors() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for group, group_commits in commits | group_by(attribute="group") %}
### {{ group }}
{% for commit in group_commits %}
- {{ commit.message }} <REPO>
{% endfor %}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[[changelog.postprocessors]]
pattern = "<REPO>"
replace = "https://github.com/test/repo"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("https://github.com/test/repo"),
        "Changelog should have postprocessor replacement applied, got: {stdout}"
    );
    assert!(
        !stdout.contains("<REPO>"),
        "Changelog should NOT contain unprocessed <REPO> placeholder"
    );
}

#[test]
fn test_project_ignore_config() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/public\", \"crates/internal\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "crates/public/Cargo.toml",
        r#"[package]
name = "public"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/public/src/lib.rs", "pub fn public() {}\n");
    repo.write_file(
        "crates/internal/Cargo.toml",
        r#"[package]
name = "internal"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/internal/src/lib.rs", "pub fn internal() {}\n");
    repo.commit("Initial commit");

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"

[projects.internal]
ignore = true
"##;
    write_custom_config(&repo, config);

    repo.write_file("crates/public/src/feature.rs", "pub fn feature() {}");
    repo.commit("feat(public): add public feature");

    repo.write_file("crates/internal/src/feature.rs", "pub fn feature() {}");
    repo.commit("feat(internal): add internal feature");

    let output = repo.run_belaf_command(&["status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("public"),
        "Status should show public package"
    );
}

#[test]
fn test_protect_breaking_commits() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for group, group_commits in commits | group_by(attribute="group") %}
### {{ group }}
{% for commit in group_commits %}
- {% if commit.breaking %}**BREAKING:** {% endif %}{{ commit.message }}
{% endfor %}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = true
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^chore"
skip = true

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/breaking.rs", "pub fn breaking() {}");
    repo.commit("chore!: breaking infrastructure change");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("breaking") || stdout.contains("BREAKING"),
        "Breaking commit should be protected even if chore is skipped, got: {stdout}"
    );
}

#[test]
fn test_changelog_limit_commits() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for commit in commits %}
- {{ commit.message }}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
limit_commits = 2
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    for i in 1..=5 {
        repo.write_file(
            &format!("src/feature{i}.rs"),
            &format!("pub fn feature{i}() {{}}"),
        );
        repo.commit(&format!("feat: add feature {i}"));
    }

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let feature_count = stdout.matches("add feature").count();

    assert!(
        feature_count <= 2,
        "Changelog should contain at most 2 commits with limit_commits=2, found {feature_count}"
    );
}

#[test]
fn test_repo_analysis_cache_sizes() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 1024
tree_cache_size = 10

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command should succeed with custom cache sizes: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_changelog_trim_false() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = """
# Changelog

"""
body = """

## {{ version }}

{% for commit in commits %}
- {{ commit.message }}
{% endfor %}

"""
trim = false
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("# Changelog") || stdout.contains("\n\n"),
        "With trim=false, whitespace should be preserved"
    );
}

#[test]
fn test_changelog_footer() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for commit in commits %}
- {{ commit.message }}
{% endfor %}
"""
footer = """

---
Generated by belaf
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Generated by belaf"),
        "Changelog should contain footer text, got: {stdout}"
    );
}

#[test]
fn test_conventional_commits_false() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for commit in commits %}
- {{ commit.message }}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = false
protect_breaking_commits = false
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = ".*"
group = "Changes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("Added a new feature without conventional format");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Added a new feature"),
        "With conventional_commits=false, non-conventional commits should be included, got: {stdout}"
    );
}

#[test]
fn test_commit_preprocessors() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for commit in commits %}
- {{ commit.message }}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_preprocessors]]
pattern = "\\(#(\\d+)\\)"
replace = "[PR #$1]"

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature (#123)");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[PR #123]") || stdout.contains("#123"),
        "Changelog should have preprocessor replacement applied, got: {stdout}"
    );
}

#[test]
fn test_link_parsers() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = """
## {{ version }}
{% for commit in commits %}
- {{ commit.message }}{% for link in commit.links %} [{{ link.text }}]({{ link.href }}){% endfor %}
{% endfor %}
"""
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.link_parsers]]
pattern = "#(\\d+)"
href = "https://github.com/test/repo/issues/$1"
text = "#$1"

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature fixes #42");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("github.com/test/repo/issues/42") || stdout.contains("#42"),
        "Changelog should have link parser applied, got: {stdout}"
    );
}

#[test]
fn test_tag_pattern() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
tag_pattern = "^v[0-9]+"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command should succeed with tag_pattern config: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_skip_tags() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
skip_tags = "beta|alpha|rc"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command should succeed with skip_tags config: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_ignore_tags() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
ignore_tags = "nightly|dev"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);

    assert!(
        output.status.success(),
        "Command should succeed with ignore_tags config: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_bump_initial_tag() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        r#"[package]
name = "brand-new-crate"
version = "0.0.0"
edition = "2021"
"#,
    );
    repo.write_file("src/lib.rs", "pub fn hello() {}\n");
    repo.commit("Initial commit");

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = false
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "1.0.0"

[commit_attribution]
strategy = "scope_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: initial feature");

    let output = repo.run_belaf_command(&["status"]);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_commit_attribution_path_first() {
    let repo = TestRepo::new();

    repo.write_file(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/api\", \"crates/web\"]\nresolver = \"2\"\n",
    );
    repo.write_file(
        "crates/api/Cargo.toml",
        r#"[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/api/src/lib.rs", "pub fn api() {}\n");
    repo.write_file(
        "crates/web/Cargo.toml",
        r#"[package]
name = "web"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write_file("crates/web/src/lib.rs", "pub fn web() {}\n");
    repo.commit("Initial commit");

    let _ = repo.run_belaf_command(&["init", "--force"]);

    let config = r##"[repo]
upstream_urls = []

[repo.analysis]
commit_cache_size = 512
tree_cache_size = 3

[changelog]
header = "# Changelog"
body = "{{ version }}"
trim = true
output = "CHANGELOG.md"
conventional_commits = true
protect_breaking_commits = true
filter_unconventional = false
filter_commits = false
sort_commits = "oldest"
include_breaking_section = true
include_contributors = false
include_statistics = false
emoji_groups = false

[[changelog.commit_parsers]]
message = "^feat"
group = "Features"

[[changelog.commit_parsers]]
message = "^fix"
group = "Bug Fixes"

[bump]
features_always_bump_minor = true
breaking_always_bump_major = true
initial_tag = "0.1.0"

[commit_attribution]
strategy = "path_first"
scope_matching = "smart"
"##;
    write_custom_config(&repo, config);

    repo.write_file("crates/web/src/feature.rs", "pub fn feature() {}");
    repo.commit("feat(api): add feature with api scope but web file change");

    let output = repo.run_belaf_command(&["status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("web"),
        "With path_first strategy, commit should be attributed to 'web' based on file path, got: {stdout}"
    );
}

#[test]
fn test_changelog_sha_links_format() {
    let repo = TestRepo::new();
    setup_basic_cargo_project(&repo);

    let _ = repo.run_belaf_command(&["init", "--force"]);

    repo.write_file("src/feature.rs", "pub fn feature() {}");
    repo.commit("feat: add new feature");

    let output = repo.run_belaf_command(&["changelog", "--preview"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let sha_link_pattern =
        regex::Regex::new(r"\(\[([a-f0-9]{7})\]\(/commit/([a-f0-9]{40})\)\)").unwrap();

    assert!(
        sha_link_pattern.is_match(&stdout),
        "Expected changelog to contain SHA links in format ([short_sha](/commit/full_sha)), got: {stdout}"
    );

    if let Some(captures) = sha_link_pattern.captures(&stdout) {
        let short_sha = captures.get(1).unwrap().as_str();
        let full_sha = captures.get(2).unwrap().as_str();
        assert!(
            full_sha.starts_with(short_sha),
            "Short SHA ({short_sha}) should be prefix of full SHA ({full_sha})"
        );
    }
}
