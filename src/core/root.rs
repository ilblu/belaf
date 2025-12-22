use crate::utils::theme::{dimmed, highlight, url, warning_message};
use serde::Deserialize;
use std::env;

#[derive(Deserialize)]
struct LatestRelease {
    #[serde(default)]
    tag_name: Option<String>,
    version: String,
    html_url: String,
}

pub fn pre_execute() {
    check_for_updates();
}

pub fn check_for_updates() {
    let current_version = env!("CARGO_PKG_VERSION");

    let result = std::thread::spawn(move || {
        let api_url =
            std::env::var("BELAF_API_URL").unwrap_or_else(|_| "https://api.belaf.dev".to_string());

        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(2)))
            .build();
        let agent: ureq::Agent = config.into();

        let mut response = match agent
            .get(&format!("{}/api/cli/releases/latest", api_url))
            .header("User-Agent", &format!("belaf/{}", current_version))
            .call()
        {
            Ok(r) => r,
            Err(_) => return,
        };

        let latest: LatestRelease = match response.body_mut().read_json() {
            Ok(r) => r,
            Err(_) => return,
        };

        if latest.version != current_version {
            let release_tag = latest.tag_name.as_deref().unwrap_or(&latest.version);
            eprintln!(
                "\n{} {} → {} ({})\n{} {}\n",
                warning_message("Update available"),
                dimmed(current_version),
                highlight(&latest.version),
                dimmed(release_tag),
                dimmed("Release:"),
                url(&latest.html_url)
            );
        }
    })
    .join();

    let _ = result;
}
