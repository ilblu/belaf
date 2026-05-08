//! `belaf doctor` — environment diagnostic. Answers "what's wrong /
//! what's set up?" without requiring the user to read 5 separate
//! commands and join the output in their head.
//!
//! Default output is human-readable text. Pass `--json` for agent
//! consumption: the JSON payload is keyed by check name with a
//! `status` field per check (`ok` | `warn` | `error` | `skipped`)
//! plus an overall `ok` boolean.

use anyhow::Result;
use owo_colors::OwoColorize;
use serde::Serialize;

use crate::core::api::{ApiClient, ApiError};
use crate::core::auth::token::load_token;
use crate::core::config::ConfigurationFile;

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckStatus {
    Ok,
    Warn,
    Error,
    Skipped,
}

impl CheckStatus {
    fn icon(&self) -> &'static str {
        match self {
            Self::Ok => "✓",
            Self::Warn => "!",
            Self::Error => "✗",
            Self::Skipped => "·",
        }
    }
}

#[derive(Serialize)]
struct Check {
    status: CheckStatus,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    /// HTTP probe latency, populated only by the `api` check.
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u128>,
}

impl Check {
    fn ok(summary: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Ok,
            summary: summary.into(),
            detail: None,
            latency_ms: None,
        }
    }
    fn warn(summary: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Warn,
            summary: summary.into(),
            detail: None,
            latency_ms: None,
        }
    }
    fn error(summary: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Error,
            summary: summary.into(),
            detail: None,
            latency_ms: None,
        }
    }
    fn skipped(summary: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Skipped,
            summary: summary.into(),
            detail: None,
            latency_ms: None,
        }
    }
    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    fn with_latency(mut self, ms: u128) -> Self {
        self.latency_ms = Some(ms);
        self
    }
    fn is_blocker(&self) -> bool {
        matches!(self.status, CheckStatus::Error)
    }
}

#[derive(Serialize)]
struct DoctorReport {
    /// Overall: false if any check is `error`. `warn` does NOT flip this.
    ok: bool,
    auth: Check,
    config: Check,
    repository: Check,
    ecosystems: Check,
    api: Check,
    environment: EnvironmentReport,
}

#[derive(Serialize)]
struct EnvironmentReport {
    api_url: String,
    api_url_overridden: bool,
    web_url: String,
    web_url_overridden: bool,
    keyring_disabled: bool,
    ci_detected: bool,
    /// Names of CI-related env vars actually present, for diagnostic
    /// transparency (so the agent can see *why* CI was detected).
    ci_signals: Vec<&'static str>,
}

const DEFAULT_API_URL: &str = "https://api.belaf.dev";
const DEFAULT_WEB_URL: &str = "https://belaf.dev/dashboard";

pub async fn run(json: bool) -> Result<i32> {
    let report = build_report().await;
    if json {
        let s = serde_json::to_string_pretty(&report)?;
        println!("{}", s);
    } else {
        render_text(&report);
    }
    if report.ok {
        Ok(0)
    } else {
        Ok(crate::core::exit_code::ExitCode::Precondition.into())
    }
}

async fn build_report() -> DoctorReport {
    let environment = collect_environment();

    // === Auth ===
    let auth = match load_token() {
        Ok(Some(token)) if !token.is_expired() => {
            // Token present and not locally expired — verify with API.
            match ApiClient::try_new() {
                Ok(client) => match client.get_user_info(&token).await {
                    Ok(user) => Check::ok(format!(
                        "authenticated as `{}`",
                        user.display_name()
                    )),
                    Err(ApiError::Unauthorized) => Check::error(
                        "token rejected by API (expired or revoked); run `belaf install`",
                    ),
                    Err(e) => Check::warn(format!(
                        "token present but API check failed: {e}"
                    )),
                },
                Err(e) => Check::warn(format!(
                    "token present but cannot construct API client: {e}"
                )),
            }
        }
        Ok(Some(_)) => Check::error("token present but locally expired; run `belaf install`"),
        Ok(None) if environment.keyring_disabled => Check::skipped(
            "keyring disabled (BELAF_NO_KEYRING set); auth check skipped — set up auth via OIDC env vars or `belaf install` in an interactive shell",
        ),
        Ok(None) => Check::error("no token in keyring; run `belaf install` to authenticate"),
        Err(e) => Check::error(format!("token storage error: {e}")),
    };

    // === Config ===
    let cfg_path = std::path::Path::new("belaf/config.toml");
    let (config, config_loaded) = if !cfg_path.exists() {
        (
            Check::warn("`belaf/config.toml` not found; run `belaf init` to create one"),
            None,
        )
    } else {
        match ConfigurationFile::get(cfg_path) {
            Ok(cfg) => (
                Check::ok(format!(
                    "`belaf/config.toml` parsed ({} release_unit blocks)",
                    cfg.release_units.len()
                )),
                Some(cfg),
            ),
            Err(e) => (
                Check::error(format!("`belaf/config.toml` invalid: {e}")),
                None,
            ),
        }
    };

    // === Repository ===
    let repository = match crate::core::git::repository::Repository::open(".") {
        Ok(_) => Check::ok("inside a git repository"),
        Err(e) => Check::error(format!("not in a git repository (or git error): {e}")),
    };

    // === Ecosystems (auto-detect) ===
    // Cheap when the repo is small, but it walks the index — only run
    // if config loaded successfully (otherwise the answer is meaningless).
    let ecosystems = match (
        config_loaded.is_some(),
        crate::core::git::repository::Repository::open("."),
    ) {
        (true, Ok(repo)) => {
            use crate::core::ecosystem::format_handler::{
                FormatHandlerRegistry, WorkspaceDiscovererRegistry,
            };
            use crate::core::release_unit::discovery::discover_implicit_release_units;
            let handlers = FormatHandlerRegistry::with_defaults();
            let discoverers = WorkspaceDiscovererRegistry::with_defaults();
            match discover_implicit_release_units(&repo, &handlers, &discoverers, &[]) {
                Ok(units) => {
                    let by_eco = ecosystem_breakdown(&units);
                    Check::ok(format!("{} ReleaseUnit(s) auto-detected", units.len()))
                        .with_detail(by_eco)
                }
                Err(e) => Check::warn(format!("auto-detect failed: {e}")),
            }
        }
        (false, _) => Check::skipped("config invalid or missing — auto-detect skipped"),
        (_, Err(e)) => Check::skipped(format!("not in a git repo — auto-detect skipped ({e})")),
    };

    // === API connectivity ===
    let api = probe_api_health(&environment.api_url).await;

    let ok = !auth.is_blocker()
        && !config.is_blocker()
        && !repository.is_blocker()
        && !ecosystems.is_blocker()
        && !api.is_blocker();

    DoctorReport {
        ok,
        auth,
        config,
        repository,
        ecosystems,
        api,
        environment,
    }
}

/// HTTP probe against `<api_url>/health`. Short timeout (3s) so the
/// doctor command stays snappy even when the network is degraded.
/// Translates concrete failures into action-oriented messages so an
/// agent reading the JSON can decide whether to retry, switch URL, or
/// surface an outage to the user.
async fn probe_api_health(base_url: &str) -> Check {
    if ApiClient::try_new().is_err() {
        return Check::error(format!("invalid API URL: {base_url}"));
    }

    let url = format!("{}/health", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Check::warn(format!("could not build HTTP client: {e}")),
    };

    let started = std::time::Instant::now();
    let result = client.get(&url).send().await;
    let elapsed_ms = started.elapsed().as_millis();

    match result {
        Ok(resp) => {
            let status = resp.status();
            // Treat any 2xx as healthy. Drill into the JSON body for the
            // `services.redis` substructure so a degraded backend still
            // surfaces as `warn` rather than a silent `ok`.
            if status.is_success() {
                let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
                let body_status = body.get("status").and_then(|v| v.as_str());
                match body_status {
                    Some("ok") => {
                        Check::ok(format!("api reachable ({}ms)", elapsed_ms)).with_latency(elapsed_ms)
                    }
                    Some("degraded") => Check::warn(format!(
                        "api reachable but reports degraded backend ({}ms); see body.services for details",
                        elapsed_ms
                    ))
                    .with_latency(elapsed_ms),
                    _ => Check::ok(format!("api reachable, no status field in body ({}ms)", elapsed_ms))
                        .with_latency(elapsed_ms),
                }
            } else {
                Check::error(format!(
                    "api responded {} from {} ({}ms)",
                    status, url, elapsed_ms
                ))
                .with_latency(elapsed_ms)
            }
        }
        Err(e) if e.is_timeout() => {
            let _ = e;
            Check::warn("api probe timed out after 3s — try again or override BELAF_API_URL")
        }
        Err(e) if e.is_connect() => Check::error(format!(
            "could not connect to {base_url} ({e}); check network or BELAF_API_URL"
        )),
        Err(e) => Check::warn(format!("api probe failed: {e}")),
    }
}

fn collect_environment() -> EnvironmentReport {
    let api_url = std::env::var("BELAF_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.to_string());
    let api_url_overridden = std::env::var("BELAF_API_URL").is_ok();
    let web_url = std::env::var("BELAF_WEB_URL").unwrap_or_else(|_| DEFAULT_WEB_URL.to_string());
    let web_url_overridden = std::env::var("BELAF_WEB_URL").is_ok();
    let keyring_disabled = std::env::var("BELAF_NO_KEYRING").is_ok();

    const CI_VARS: &[&str] = &[
        "CI",
        "GITHUB_ACTIONS",
        "GITLAB_CI",
        "BUILDKITE",
        "CIRCLECI",
        "JENKINS_URL",
    ];
    let ci_signals: Vec<&'static str> = CI_VARS
        .iter()
        .filter(|v| std::env::var(v).is_ok())
        .copied()
        .collect();
    let ci_detected = !ci_signals.is_empty();

    EnvironmentReport {
        api_url,
        api_url_overridden,
        web_url,
        web_url_overridden,
        keyring_disabled,
        ci_detected,
        ci_signals,
    }
}

fn ecosystem_breakdown(units: &[crate::core::ecosystem::format_handler::DiscoveredUnit]) -> String {
    use std::collections::BTreeMap;
    let mut by_eco: BTreeMap<&str, usize> = BTreeMap::new();
    for u in units {
        let eco = u.qnames.get(1).map(String::as_str).unwrap_or("(unknown)");
        *by_eco.entry(eco).or_insert(0) += 1;
    }
    by_eco
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_text(r: &DoctorReport) {
    println!("{}", "belaf doctor".bold());
    println!();
    render_check("auth        ", &r.auth);
    render_check("config      ", &r.config);
    render_check("repository  ", &r.repository);
    render_check("ecosystems  ", &r.ecosystems);
    render_check("api         ", &r.api);
    println!();
    println!("{}", "environment".bold());
    println!(
        "  api_url      : {}{}",
        r.environment.api_url,
        if r.environment.api_url_overridden {
            " (overridden via BELAF_API_URL)"
        } else {
            ""
        }
    );
    println!(
        "  web_url      : {}{}",
        r.environment.web_url,
        if r.environment.web_url_overridden {
            " (overridden via BELAF_WEB_URL)"
        } else {
            ""
        }
    );
    println!(
        "  keyring      : {}",
        if r.environment.keyring_disabled {
            "disabled (BELAF_NO_KEYRING=1)"
        } else {
            "enabled"
        }
    );
    println!(
        "  ci_detected  : {}{}",
        r.environment.ci_detected,
        if r.environment.ci_signals.is_empty() {
            "".to_string()
        } else {
            format!(" ({})", r.environment.ci_signals.join(", "))
        }
    );
    println!();
    if r.ok {
        println!("{} {}", "✓".green(), "ready".bold());
    } else {
        println!("{} {}", "✗".red(), "not ready".bold().red());
        println!(
            "  Run with `--json` for machine-readable details, or address the `✗` items above."
        );
    }
}

fn render_check(label: &str, check: &Check) {
    let icon = check.status.icon();
    let coloured = match check.status {
        CheckStatus::Ok => icon.green().to_string(),
        CheckStatus::Warn => icon.yellow().to_string(),
        CheckStatus::Error => icon.red().to_string(),
        CheckStatus::Skipped => icon.dimmed().to_string(),
    };
    println!("  {coloured} {label} {}", check.summary);
    if let Some(detail) = &check.detail {
        println!("                    {}", detail.dimmed());
    }
}
