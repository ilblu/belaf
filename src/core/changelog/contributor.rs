use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct RemoteContributor {
    pub username: Option<String>,
    pub pr_title: Option<String>,
    pub pr_number: Option<i64>,
    pub pr_labels: Vec<String>,
    pub is_first_time: bool,
}

impl Hash for RemoteContributor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.username.hash(state);
    }
}
