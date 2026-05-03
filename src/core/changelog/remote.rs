use dyn_clone::DynClone;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::contributor::RemoteContributor;

pub const MAX_PAGE_SIZE: usize = 100;

pub trait RemoteCommit: DynClone + Send + Sync {
    fn id(&self) -> String;
    fn username(&self) -> Option<String>;
    fn timestamp(&self) -> Option<i64>;

    fn convert_to_unix_timestamp(&self, date: &str) -> i64 {
        OffsetDateTime::parse(date, &Rfc3339)
            .expect("failed to parse date")
            .unix_timestamp()
    }
}

dyn_clone::clone_trait_object!(RemoteCommit);

pub trait RemotePullRequest: DynClone + Send + Sync {
    fn number(&self) -> i64;
    fn title(&self) -> Option<String>;
    fn labels(&self) -> Vec<String>;
    fn merge_commit(&self) -> Option<String>;
}

dyn_clone::clone_trait_object!(RemotePullRequest);

pub type RemoteMetadata = (Vec<Box<dyn RemoteCommit>>, Vec<Box<dyn RemotePullRequest>>);

#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct RemoteReleaseMetadata {
    pub contributors: Vec<RemoteContributor>,
}
