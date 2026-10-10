//! Shared W04 destination fixtures: the manifest bytes every commit test
//! uses, the artifacts root path helper, an independent SHA-256 over exact
//! bytes, and the object-write helper the staging tests share.

use std::{io::Write, path::PathBuf};

use saya_harness::coding::snapshot::{SnapshotError, StagedSnapshot};
use saya_types::Sha256Digest;

use super::super::support::TempRoot;

pub(super) const MANIFEST_BYTES: &[u8] = b"{\"snapshot\":\"destination fixture\"}";

pub(super) fn artifacts_root(temp: &TempRoot) -> PathBuf {
    temp.path().join("artifacts")
}

/// An independent SHA-256 over the exact bytes, so no expected digest is
/// ever derived from the code under test.
pub(super) fn digest(bytes: &[u8]) -> Sha256Digest {
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;
    let mut hex = String::new();
    for byte in Sha256::digest(bytes) {
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    Sha256Digest::parse(&hex).unwrap()
}

pub(super) fn write_object(staged: &mut StagedSnapshot, bytes: &[u8]) -> Result<(), SnapshotError> {
    let mut sink = staged.new_object()?;
    // `SnapshotError` has no `From<io::Error>`, so the write failure is
    // mapped explicitly instead of using `?`.
    sink.write_all(bytes)
        .map_err(|error| SnapshotError::CaptureIo {
            description: format!("write staged object: {error}"),
        })?;
    sink.finish(&digest(bytes))
}
