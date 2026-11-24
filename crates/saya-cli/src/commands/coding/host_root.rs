//! Resolving the one in-place root an application works over: the
//! dangerous-root refusal first, then the canonical directory identity.

use super::CodingError;
use saya_harness::coding::local::LocalRoot;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Resolves one in-place root: the dangerous-root refusal first, then the
/// canonical directory identity (which refuses a symlink, a file, a missing
/// path, and either direction of state overlap).
pub(crate) fn resolve_in_place(
    stated: &Path,
    canonical: &Path,
    state_root: &Path,
) -> Result<LocalRoot, CodingError> {
    refuse_dangerous_root(canonical, state_root)?;
    LocalRoot::open(stated, state_root).map_err(|error| CodingError::InPlaceRoot {
        detail: error.to_string(),
    })
}

/// Resolves the root a session over a folder that cannot root an in-place job
/// opens over: the folder's own canonical identity, taken without the
/// dangerous-root refusal because nothing here is a job root — no baseline is
/// inventoried and no file is reached. The state root is not asked of it
/// either: a run with no workspace composes no file, shell or write tool, so a
/// state root the configuration keeps inside the folder is reachable by
/// nothing. What it still refuses is everything about the folder itself that
/// [`LocalRoot::label`] refuses (a symlink, a file, a missing path).
pub(crate) fn resolve_session_root(
    stated: &Path,
    _state_root: &Path,
) -> Result<LocalRoot, CodingError> {
    LocalRoot::label(stated).map_err(|error| CodingError::InPlaceRoot {
        detail: error.to_string(),
    })
}

/// Refuses the roots an in-place job must never adopt: the filesystem root,
/// the user's home directory, and saya's own state root. A folder below home
/// — a container workdir on a volume, a project directory — is admitted.
pub(super) fn refuse_dangerous_root(path: &Path, state_root: &Path) -> Result<(), CodingError> {
    let reason = if path == Path::new("/") {
        Some("it is the filesystem root")
    } else if home_directory().as_deref() == Some(path) {
        Some("it is the home directory")
    } else if path == state_root {
        Some("it is saya's state directory")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(CodingError::InPlaceUnsupportedRoot { reason }),
        None => Ok(()),
    }
}

pub(super) fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|home| fs::canonicalize(home).ok())
}
