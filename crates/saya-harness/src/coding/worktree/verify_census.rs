//! A census of the in-place roots this process has verified.
//!
//! An in-place root is verified at use time, and one composition verifies it
//! once: the consumer that verified the root binds the [`VerifiedCheckout`] it
//! got and hands it on, instead of letting the next layer read the folder and
//! its baseline again. That is a claim about work done twice, so it is counted
//! here and asserted where the compositions are tested (`saya-cli`), rather
//! than only stated in prose.
//!
//! The count is per root path, so a test reads the count for the folder it
//! made and no other test's composition can be mistaken for its own. A process
//! verifies a handful of roots; the map is one small entry per root.
//!
//! [`VerifiedCheckout`]: super::VerifiedCheckout

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

/// One entry per verified root, keyed by the root's own path.
static CENSUS: Mutex<BTreeMap<PathBuf, u64>> = Mutex::new(BTreeMap::new());

/// Records one verification of the in-place root at `root`.
pub(super) fn note(root: &Path) {
    let mut census = CENSUS.lock().unwrap_or_else(PoisonError::into_inner);
    *census.entry(root.to_path_buf()).or_insert(0) += 1;
}

/// How many times this process has verified the in-place root at `root`.
///
/// One composition is one verification: a caller that verifies a root and then
/// hands the proof on leaves this at one per composition.
pub fn in_place_verifications(root: &Path) -> u64 {
    let census = CENSUS.lock().unwrap_or_else(PoisonError::into_inner);
    census.get(root).copied().unwrap_or(0)
}
