use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::release::Release;

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkCount {
    pub text: String,
    pub href: String,
    pub count: usize,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Statistics {
    pub commit_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commits_timespan: Option<i64>,
    pub conventional_commit_count: usize,
    pub links: Vec<LinkCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_passed_since_last_release: Option<i64>,
}

impl From<&Release<'_>> for Statistics {
    fn from(release: &Release) -> Self {
        let commit_count = release.commits.len();

        let commits_timespan = if release.commits.len() < 2 {
            log::trace!(
                "Insufficient commits to calculate duration (found {})",
                release.commits.len()
            );
            None
        } else {
            release
                .commits
                .iter()
                .min_by_key(|c| c.committer.timestamp)
                .zip(
                    release
                        .commits
                        .iter()
                        .max_by_key(|c| c.committer.timestamp),
                )
                .and_then(|(first, last)| {
                    OffsetDateTime::from_unix_timestamp(first.committer.timestamp)
                        .ok()
                        .zip(
                            OffsetDateTime::from_unix_timestamp(last.committer.timestamp).ok(),
                        )
                        .map(|(start, end)| {
                            let start_date = start.date();
                            let end_date = end.date();
                            (end_date - start_date).whole_days()
                        })
                })
        };

        let conventional_commit_count =
            release.commits.iter().filter(|c| c.conv.is_some()).count();

        let mut links: Vec<LinkCount> = release
            .commits
            .iter()
            .fold(HashMap::new(), |mut acc, c| {
                for link in &c.links {
                    *acc.entry((link.text.clone(), link.href.clone()))
                        .or_insert(0) += 1;
                }
                acc
            })
            .into_iter()
            .map(|((text, href), count)| LinkCount { text, href, count })
            .collect();

        links.sort_by(|lhs, rhs| {
            rhs.count
                .cmp(&lhs.count)
                .then_with(|| lhs.text.cmp(&rhs.text))
                .then_with(|| lhs.href.cmp(&rhs.href))
        });

        let days_passed_since_last_release = match release.previous.as_ref() {
            Some(prev) => {
                let curr_ts = release
                    .timestamp
                    .and_then(|ts| OffsetDateTime::from_unix_timestamp(ts).ok())
                    .unwrap_or_else(OffsetDateTime::now_utc);

                prev.timestamp
                    .and_then(|ts| OffsetDateTime::from_unix_timestamp(ts).ok())
                    .map(|prev_dt| {
                        let curr_date = curr_ts.date();
                        let prev_date = prev_dt.date();
                        (curr_date - prev_date).whole_days()
                    })
            }
            None => {
                log::trace!("Previous release not found");
                None
            }
        };

        Self {
            commit_count,
            commits_timespan,
            conventional_commit_count,
            links,
            days_passed_since_last_release,
        }
    }
}
