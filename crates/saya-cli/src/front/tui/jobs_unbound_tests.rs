//! `/start` from a session that bound no workspace: a job is a job over a
//! folder, and this session has none, so the surface refuses it in one
//! sentence instead of rooting the job in whatever directory the process
//! happens to stand in.

use std::sync::Arc;

use saya_agent::ApprovalPolicy;

use super::direct_fixture::Fixture;
use super::jobs_open::{JobDeps, NO_WORKSPACE};
use super::run_front::Forward;
use crate::cli::CodingCommand;

/// One `/start`, as the surface states it: a job with no stated write scope
/// (a session start may edit the whole workspace) run in place.
fn start() -> CodingCommand {
    CodingCommand::Start {
        id: String::from("unbound-job"),
        task: String::from("count the rows"),
        write_paths: Vec::new(),
        database_profiles: Vec::new(),
        tools: Vec::new(),
        plan: false,
        approve: false,
        in_place: true,
        managed: false,
    }
}

/// A front over a queue this test holds: the refusal is the front's own
/// return, decided before any worker is opened.
fn front() -> (
    Forward,
    tokio::sync::mpsc::UnboundedReceiver<super::msg::Msg>,
) {
    let (msgs, queue) = super::channel();
    (Forward::new(msgs), queue)
}

#[test]
fn a_start_from_an_unbound_session_is_refused_in_one_sentence() {
    let fixture = Fixture::new("unbound-start");
    // The launch's own call: `session.root()` is `None` for a session that
    // answered the trust question `n`, and that is what the surface hands on.
    let deps = JobDeps::new(
        Arc::clone(&fixture.deps.runtime),
        fixture.deps.state_db.clone(),
        None,
        "unbound-session",
        ApprovalPolicy::Ask,
    );
    assert!(
        deps.folder().is_none(),
        "a session that bound no folder hands the job none: {:?}",
        deps.folder()
    );
    let (front, _queue) = front();
    let refusal = front.job_command(start(), &deps);
    assert_eq!(
        refusal,
        Some(NO_WORKSPACE),
        "a job from an unbound session is refused in one sentence, never rooted elsewhere"
    );
    assert!(
        NO_WORKSPACE.contains("no workspace") && NO_WORKSPACE.contains("`y`"),
        "the refusal says what is missing and how to answer it: {NO_WORKSPACE}"
    );
}

#[test]
fn a_start_from_a_session_with_a_folder_still_opens_its_job() {
    let fixture = Fixture::new("bound-start");
    let deps = JobDeps::new(
        Arc::clone(&fixture.deps.runtime),
        fixture.deps.state_db.clone(),
        Some(&fixture.root),
        "bound-session",
        ApprovalPolicy::Ask,
    );
    assert_eq!(
        deps.folder(),
        Some(fixture.root.as_path()),
        "a session with a folder opens its job over that folder"
    );
}
