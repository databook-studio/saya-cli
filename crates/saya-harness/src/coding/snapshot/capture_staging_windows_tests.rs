//! Unit tests for the Windows publish boundary: the rename must act on a
//! staging directory with no handle this stage still holds inside it. Real
//! Windows refuses such a rename with `ERROR_ACCESS_DENIED` (os error 5);
//! Wine allows it, so the invariant is asserted structurally at the boundary
//! rather than through the OS refusal.

use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use saya_types::SnapshotId;

use super::super::super::{
    SnapshotArtifacts,
    capture_lock_windows::{StageLock, probe_stage_lock},
    manifest,
};
use super::set_before_publish_observer;

/// Removes the temporary artifacts root this test created on drop.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "saya-{label}-{}-{}",
            std::process::id(),
            crate::workspace::contain::nanos()
        ));
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// At the publish rename the stage must hold no handle inside the staging
/// directory. The stage lock is the only handle this stage keeps there, so
/// it must be closed — and the lock file it left behind must be free for the
/// sweep's own probe, which is why commit holds the root lock across the
/// release and the rename.
#[test]
fn publish_releases_every_handle_inside_the_staging_directory() {
    let temp = TempRoot::new("publish-handles");
    let artifacts = SnapshotArtifacts::open(temp.path()).unwrap();
    let id = SnapshotId::parse("publish-handles").unwrap();
    let manifest_bytes = br#"{"entries":[]}"#;
    let manifest_digest = manifest::digest(manifest_bytes).unwrap();

    let observed = Rc::new(Cell::new(false));
    let record = Rc::clone(&observed);
    set_before_publish_observer(Box::new(move |stage| {
        record.set(true);
        assert!(
            stage.lock_file.is_none(),
            "the stage lock handle is still held at the publish rename"
        );
        match probe_stage_lock(&stage.staging_path) {
            StageLock::Free(lock) => drop(lock),
            StageLock::Held => panic!("the stage lock is still held at the publish rename"),
            StageLock::Absent => panic!("the stage lock file is missing at the publish rename"),
        }
    }));

    let staged = artifacts.begin(&id).unwrap();
    staged.commit(manifest_bytes, &manifest_digest).unwrap();

    assert!(observed.get(), "the publish boundary observer must run");
    assert!(artifacts.is_complete(&id).unwrap());
    // The released lock is unlinked after the rename: it never publishes.
    assert!(!temp.path().join(id.as_str()).join("lock").exists());
}
