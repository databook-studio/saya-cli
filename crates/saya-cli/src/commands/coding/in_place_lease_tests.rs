//! The lease a given-up prepared run leaves behind, and whose folder it makes.
//!
//! The journey's first process states a preference (`/preferences add …`)
//! before its first line. That is an input a run is composed from, so the run
//! the loop prepared at the prompt is given up and the first plain line
//! composes its own — but the run that was given up had already leased the
//! folder, under this session's name, and nothing drives it any more.
//!
//! A lease naming the session asking is that session's own folder: a job
//! stating the session is admitted beside it (`InPlaceLease::acquire`), so the
//! run the next line composes writes the folder rather than reading it as
//! another owner's.
#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use saya_harness::coding::local::LocalRoot;

use super::in_place_lease::InPlaceLease;
use crate::chat::{WorkspaceShape, workspace_shape};

/// The session the prepared run leased the folder for.
const SESSION: &str = "session-preference-first";

/// A throwaway base directory, canonical, removed when the case ends.
fn case(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base =
        std::env::temp_dir().join(format!("saya-own-lease-{label}-{}-{stamp}", process::id()));
    fs::create_dir_all(base.join("site")).unwrap();
    fs::create_dir_all(base.join("state")).unwrap();
    base.canonicalize().unwrap()
}

/// The folder the run the preference gave up left leased, and the session
/// stating the preference: the next line's run writes it.
#[test]
fn a_preference_stated_before_the_first_line_leaves_the_folder_the_sessions_own() {
    let base = case("preference");
    let (folder, state) = (base.join("site"), base.join("state"));
    let root = LocalRoot::open(&folder, &state).expect("the folder is an in-place root");
    // The run the loop composed at the prompt: it leased the folder for this
    // session, and the preference gave the run up without giving the lease back.
    let prepared = InPlaceLease::acquire(&state, root.id().as_str(), SESSION, None)
        .expect("the session takes the folder's lease");

    assert_eq!(
        workspace_shape(&folder, &state, SESSION),
        WorkspaceShape::Writable,
        "the session's own run leases the folder: the first line still writes it"
    );
    assert_eq!(
        workspace_shape(&folder, &state, "session-someone-else"),
        WorkspaceShape::ReadOnlyFiles {
            reason: "another session owns this folder".to_owned()
        },
        "another session reads a folder this session leases"
    );

    drop(prepared);
    assert_eq!(
        workspace_shape(&folder, &state, "session-someone-else"),
        WorkspaceShape::Writable,
        "the lease is given back, so nobody owns the folder"
    );
    let _ = fs::remove_dir_all(Path::new(&base));
}
