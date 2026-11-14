use std::{collections::BTreeSet, path::PathBuf};

use crate::coding::git::GitRepository;

use super::types::{MAX_PRIVATE_PREFIX_BYTES, MAX_PRIVATE_PREFIXES};
use super::{SnapshotError, SnapshotPath};

pub struct SnapshotOmissionPolicy {
    pub(super) root: PathBuf,
    pub(super) common_dir: PathBuf,
    pub(super) private_prefixes: Vec<String>,
}

impl SnapshotOmissionPolicy {
    pub fn new(
        repository: &GitRepository,
        private_prefixes: Vec<String>,
    ) -> Result<Self, SnapshotError> {
        let prefix_bytes = private_prefixes
            .iter()
            .try_fold(0usize, |total, prefix| total.checked_add(prefix.len()))
            .ok_or(SnapshotError::TooLarge)?;
        if private_prefixes.len() > MAX_PRIVATE_PREFIXES || prefix_bytes > MAX_PRIVATE_PREFIX_BYTES
        {
            return Err(SnapshotError::TooLarge);
        }
        let mut prefixes = BTreeSet::new();
        for prefix in private_prefixes {
            let path = SnapshotPath::parse(prefix)?;
            prefixes.insert(path.as_str().to_owned());
        }
        Ok(Self {
            root: repository.root().to_path_buf(),
            common_dir: repository.common_dir().to_path_buf(),
            private_prefixes: prefixes.into_iter().collect(),
        })
    }
}

impl std::fmt::Debug for SnapshotOmissionPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SnapshotOmissionPolicy { .. }")
    }
}
