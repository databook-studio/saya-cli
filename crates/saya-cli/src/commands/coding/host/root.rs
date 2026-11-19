use saya_harness::coding::{git::GitRepository, local::LocalRoot};
use std::path::Path;

/// The one root a coding application works over: a Git repository's working
/// tree (today's managed-checkout mode) or a plain folder with a captured
/// local baseline. The variant decides every Git-only operation; an in-place
/// root refuses it rather than silently creating a checkout.
pub(in crate::commands::coding) enum CodingRoot {
    Git(GitRepository),
    InPlace(LocalRoot),
}

impl CodingRoot {
    pub(in crate::commands::coding) fn path(&self) -> &Path {
        match self {
            Self::Git(repository) => repository.root(),
            Self::InPlace(local) => local.path(),
        }
    }

    pub(in crate::commands::coding) fn git(&self) -> Option<&GitRepository> {
        match self {
            Self::Git(repository) => Some(repository),
            Self::InPlace(_) => None,
        }
    }

    pub(in crate::commands::coding) fn local(&self) -> Option<&LocalRoot> {
        match self {
            Self::Git(_) => None,
            Self::InPlace(local) => Some(local),
        }
    }
}
