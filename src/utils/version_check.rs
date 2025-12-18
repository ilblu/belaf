use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

const GITHUB_API_URL: &str = "https://api.github.com/repos/ilblu/belaf/releases/latest";
const CHECK_INTERVAL_HOURS: u64 = 24;

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

#[derive(Debug, Clone, Copy)]
enum InstallMethod {
    Homebrew,
    Cargo,
    Scoop,
    Unknown,
}

impl InstallMethod {
    fn detect() -> Self {
        let current_exe = match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => return InstallMethod::Unknown,
        };

        let exe_path = current_exe.to_string_lossy();

        if exe_path.contains("/opt/homebrew/") || exe_path.contains("/usr/local/Cellar/") {
            return InstallMethod::Homebrew;
        }

        if exe_path.contains("/.cargo/bin/") {
            return InstallMethod::Cargo;
        }

        if exe_path.contains("/scoop/shims/") || exe_path.contains("\\scoop\\shims\\") {
            return InstallMethod::Scoop;
        }

        InstallMethod::Unknown
    }

    fn upgrade_command(&self) -> &'static str {
        match self {
            InstallMethod::Homebrew => "brew upgrade belaf",
            InstallMethod::Cargo => "cargo install belaf",
            InstallMethod::Scoop => "scoop update belaf",
            InstallMethod::Unknown => "https://github.com/ilblu/belaf/releases",
        }
    }

    fn is_command(&self) -> bool {
        !matches!(self, InstallMethod::Unknown)
    }
}

pub fn check_for_updates(current_version: &str, force_fetch: bool) {
    let Some(cache_path) = get_cache_path() else {
        if force_fetch {
            if let Some(latest_version) = fetch_latest_from_github() {
                if is_newer_version(&latest_version, current_version) {
                    print_update_message(&latest_version, current_version);
                }
            }
        }
        return;
    };

    if let Some(latest_version) = get_latest_version(&cache_path, force_fetch) {
        if is_newer_version(&latest_version, current_version) {
            print_update_message(&latest_version, current_version);
        }
    }
}

fn get_cache_path() -> Option<PathBuf> {
    let cache_dir = directories::ProjectDirs::from("", "", "belaf")
        .map(|d| d.cache_dir().to_path_buf())
        .or_else(|| dirs::cache_dir().map(|d| d.join("belaf")))?;

    if !cache_dir.exists() {
        let _ = fs::create_dir_all(&cache_dir);
    }

    Some(cache_dir.join("latest-version"))
}

fn should_fetch_latest(cache_path: &PathBuf, force_fetch: bool) -> bool {
    if force_fetch {
        return true;
    }

    if let Ok(metadata) = fs::metadata(cache_path) {
        if let Ok(modified) = metadata.modified() {
            if let Ok(elapsed) = SystemTime::now().duration_since(modified) {
                return elapsed.as_secs() > CHECK_INTERVAL_HOURS * 3600;
            }
        }
    }
    true
}

fn get_latest_version(cache_path: &PathBuf, force_fetch: bool) -> Option<String> {
    if should_fetch_latest(cache_path, force_fetch) {
        if let Some(version) = fetch_latest_from_github() {
            let _ = fs::write(cache_path, &version);
            return Some(version);
        }
    }

    fs::read_to_string(cache_path).ok()
}

fn fetch_latest_from_github() -> Option<String> {
    let mut response = ureq::get(GITHUB_API_URL)
        .header("User-Agent", "belaf-cli")
        .call()
        .ok()?;

    let release: GithubRelease = response.body_mut().read_json().ok()?;
    Some(release.tag_name)
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    let latest_clean = latest.trim_start_matches('v');
    let current_clean = current.trim_start_matches('v');

    match (
        semver::Version::parse(latest_clean),
        semver::Version::parse(current_clean),
    ) {
        (Ok(latest_ver), Ok(current_ver)) => latest_ver > current_ver,
        _ => false,
    }
}

fn print_update_message(latest: &str, current: &str) {
    use owo_colors::OwoColorize;

    let install_method = InstallMethod::detect();
    let upgrade = install_method.upgrade_command();

    eprintln!();
    eprintln!(
        "{}",
        "╭─────────────────────────────────────────────────────╮".bright_yellow()
    );
    eprintln!(
        "{}  {} {} → {}",
        "│".bright_yellow(),
        "Update available:".bright_yellow().bold(),
        current.dimmed(),
        latest.bright_green().bold()
    );

    if install_method.is_command() {
        eprintln!("{}  Run: {}", "│".bright_yellow(), upgrade.bright_cyan());
    } else {
        eprintln!(
            "{}  Download: {}",
            "│".bright_yellow(),
            upgrade.bright_cyan()
        );
    }

    eprintln!(
        "{}",
        "╰─────────────────────────────────────────────────────╯".bright_yellow()
    );
    eprintln!();
}
