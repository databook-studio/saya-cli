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

use super::in_place_lease::{InPlaceLease, LeaseCell};
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
    let prepared = InPlaceLease::acquire(&state, root.id().as_str(), SESSION, None, SESSION)
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

/// The job a session starts over its own folder: the session's run leases the
/// folder, and the job states the session, so it is admitted beside that lease
/// without taking one of its own.
const JOB: &str = "job-started-by-the-session";

/// The folder a session's run leased, and the job it started there.
fn session_and_job(base: &Path) -> (PathBuf, PathBuf, String, LeaseCell, LeaseCell) {
    let (folder, state) = (base.join("site"), base.join("state"));
    let root = LocalRoot::open(&folder, &state).expect("the folder is an in-place root");
    let root_id = root.id().as_str().to_owned();
    let session = InPlaceLease::acquire(&state, &root_id, SESSION, None, SESSION)
        .expect("the session's run takes the folder's lease");
    let job = InPlaceLease::acquire(&state, &root_id, SESSION, Some(SESSION), JOB)
        .expect("the job is admitted beside the session's run");
    (folder, state, root_id, session, job)
}

/// A session that ends while a dependent job still runs hands the folder to the
/// job: a second session reads it, another run cannot take it, the job's writes
/// still land, and the folder is free once the job ends.
#[test]
fn a_dependent_job_holds_the_folder_after_its_session_ends() {
    let base = case("job-handover");
    let (folder, state, root_id, session, job) = session_and_job(&base);

    // The session's run ends. The job it started is still running.
    drop(session);

    assert_eq!(
        workspace_shape(&folder, &state, "second-session"),
        WorkspaceShape::ReadOnlyFiles {
            reason: "another session owns this folder".to_owned()
        },
        "the running job holds the folder its session let go"
    );
    assert!(
        InPlaceLease::acquire(&state, &root_id, "second-session", None, "second-run").is_err(),
        "a second run cannot take the folder the running job holds"
    );
    fs::write(folder.join("job-wrote.txt"), b"job\n").expect("the job writes its folder");
    assert_eq!(
        fs::read(folder.join("job-wrote.txt")).expect("the job's write reads back"),
        b"job\n",
        "the job's writes still land"
    );

    drop(job);
    assert_eq!(
        workspace_shape(&folder, &state, "second-session"),
        WorkspaceShape::Writable,
        "the folder is free once the job ends"
    );
    let _ = fs::remove_dir_all(&base);
}

/// The crash path: the session's run is dropped without stating a close (a
/// panicking run), and the job it started still takes the folder. Nothing is
/// closed while the job runs.
#[test]
fn a_session_run_dropped_without_a_close_hands_the_folder_to_its_job() {
    let base = case("job-handover-crash");
    let (folder, state, root_id, session, job) = session_and_job(&base);

    let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // The session's run dies with this unwind; the job thread lives on.
        let _run = session;
        panic!("the session's run crashed");
    }));
    assert!(died.is_err(), "the session's run crashed");
    assert!(
        super::in_place_lease::leased(&state, &root_id).expect("the folder's lease is readable"),
        "the crashed session's job holds the folder"
    );
    assert_eq!(
        workspace_shape(&folder, &state, "second-session"),
        WorkspaceShape::ReadOnlyFiles {
            reason: "another session owns this folder".to_owned()
        },
        "a second session reads the folder the crashed session's job holds"
    );

    drop(job);
    assert!(
        !super::in_place_lease::leased(&state, &root_id).expect("the folder's lease is readable"),
        "the folder is free once the job ends"
    );
    let _ = fs::remove_dir_all(&base);
}
