//! Closed npm action and settled status contracts.

pub use super::{npm_bundle_receipt::NpmInputBundleReceipt, npm_check_receipt::NpmCheckOutcome};

/// A closed npm check command selected by trusted configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpmCheckAction {
    TestCi,
    Build,
}

/// The settled disposition of a closed npm check action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpmCheckStatus {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

pub(super) fn output_sha256(stdout: &str, stderr: &str) -> String {
    use sha2::{Digest as _, Sha256};

    let mut hasher = Sha256::new();
    for part in [stdout.as_bytes(), stderr.as_bytes()] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
