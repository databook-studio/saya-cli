//! Invalid ids are refused everywhere and touch nothing.

use super::temp_root;
use saya_harness::HarnessError;
use saya_harness::session_lifecycle::{acquire_session_lock, delete_session, ensure_not_deleted};
use saya_store::FsSessionStore;

/// f. Invalid ids are refused everywhere and touch nothing.
#[tokio::test]
async fn an_invalid_id_is_refused_everywhere() {
    let root = temp_root("f-invalid");
    let sessions = root.join("sessions");
    let store = FsSessionStore::new(&sessions);
    for id in ["../x", "a/b", ""] {
        for error in [
            ensure_not_deleted(&store, id).unwrap_err(),
            acquire_session_lock(&store, &sessions.join(id), id).unwrap_err(),
            delete_session(&store, &sessions, id).unwrap_err(),
        ] {
            assert!(
                matches!(error, HarnessError::Store { .. }),
                "expected Store for {id:?}, got {error:?}"
            );
        }
    }
    // Nothing was created: not the sessions root, not a tombstone dir.
    assert!(!sessions.exists());

    let _ = std::fs::remove_dir_all(root);
}
