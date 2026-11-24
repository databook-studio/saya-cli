//! `/sessions` and `/resume` on the **live** run: choosing a saved session
//! resumes it, and the next message is a turn of that conversation.
//!
//! Every case drives the surface's own port over a real run against a scripted
//! provider, and reads the request bodies the provider actually received — so
//! "resumed" is proved by what the next turn carried, never by a sentence the
//! surface said about itself.

#![cfg(unix)]

use std::fs;
use std::path::Path;

use super::run_cmd::CmdPort;
use super::surface_fixture::surface;

/// The record both resume cases open: the conversation the next turn must carry.
const EARLIER: [(&str, &str); 2] = [
    ("earlier question", "earlier answer"),
    ("another question", "another answer"),
];

/// The rows `/sessions` offers are the saved records, newest first, with what
/// an operator chooses one by.
#[tokio::test]
async fn bare_sessions_offers_the_saved_records_to_the_picker() {
    let fixture = surface("sessions-picker").await;
    fixture
        .save_session("s-older", &[("older question", "older answer")], None)
        .await;
    fixture
        .save_session("s-newer", &[("newer question", "newer answer")], None)
        .await;

    fixture.op("/sessions").await;
    let rows = fixture.picked().await;

    assert_eq!(
        rows.len(),
        2,
        "the two saved records are offered: {rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.title == "newer question"),
        "a row is named by what the session asked: {rows:#?}"
    );
    assert!(
        rows.iter().all(|row| !row.age.is_empty()),
        "every row states its age: {rows:#?}"
    );
}

/// `↵` on a row is the resume: the record's conversation is the one the next
/// message continues.
#[tokio::test]
async fn choosing_a_row_resumes_it_and_the_next_turn_carries_its_turns() {
    let fixture = surface("picker-resume").await;
    fixture.save_session("s-earlier", &EARLIER, None).await;
    fixture.port.send("this session's first message".to_owned());
    fixture.settle(1).await;

    let rows = fixture.picker().await;
    let chosen = rows.first().expect("the record is offered").id.clone();
    fixture.resume(&chosen).await;

    let said = fixture.said();
    assert!(
        said.iter().any(|line| line.starts_with("resumed earlie")),
        "the surface says what it resumed: {said:?}"
    );
    assert_eq!(
        fixture.state().id,
        "s-earlier",
        "the session the surface is on is the record's"
    );

    fixture.port.send("follow-up".to_owned());
    fixture.settle(2).await;
    let body = fixture.bodies().pop().expect("the resumed turn's request");
    assert!(
        body.contains("earlier question") && body.contains("earlier answer"),
        "the resumed conversation's turns are replayed: {body}"
    );
    assert!(body.contains("follow-up"), "and the turn just sent: {body}");
}

/// The same operation, stated with an id: `/resume <id>` mid-session carries
/// that session's turns, and never says "quit and restart".
#[tokio::test]
async fn resuming_with_an_id_mid_session_carries_that_sessions_turns() {
    let fixture = surface("resume-id").await;
    fixture.save_session("s-earlier", &EARLIER, None).await;
    fixture.port.send("before the resume".to_owned());
    fixture.settle(1).await;

    fixture.op("/resume s-earlier").await;
    let said = fixture.said();
    assert!(
        !said
            .iter()
            .any(|line| line.contains("quit and start") || line.contains("--resume <id>")),
        "the surface no longer sends the operator to a flag: {said:?}"
    );

    fixture.port.send("after the resume".to_owned());
    fixture.settle(2).await;
    let body = fixture.bodies().pop().expect("the resumed turn's request");
    assert!(
        body.contains("earlier question") && body.contains("after the resume"),
        "the next turn continues the resumed session: {body}"
    );
}

/// A record whose folder another live session holds is refused in one sentence,
/// and the session the surface is on keeps its run and its conversation.
#[tokio::test]
async fn a_folder_another_session_holds_refuses_the_resume() {
    let fixture = surface("resume-refused").await;
    let base = fixture
        .folder()
        .parent()
        .expect("the fixture root")
        .to_path_buf();
    let other = base.join("other-folder");
    fs::create_dir_all(&other).expect("the other folder builds");
    fixture
        .save_session("s-elsewhere", &EARLIER, Some(other.clone()))
        .await;
    // The folder's lease, held the way a second session's run holds it.
    let _held = hold_lease(&base.join("coding"), &other);

    fixture.port.send("still this session".to_owned());
    fixture.settle(1).await;

    fixture.op("/resume s-elsewhere").await;
    let said = fixture.said();
    assert!(
        said.iter()
            .any(|line| line == "cannot resume s-elsewhere: another session owns this folder"),
        "the refusal is one sentence naming the record: {said:?}"
    );
    assert_eq!(
        fixture.state().id,
        "surface-resume-refused",
        "a refused resume leaves the session where it was"
    );

    fixture
        .port
        .send("and the old session continues".to_owned());
    fixture.settle(2).await;
    let body = fixture.bodies().pop().expect("the turn after the refusal");
    assert!(
        !body.contains("earlier question"),
        "nothing of the refused record is replayed: {body}"
    );
}

/// Holds the folder's lease the way a second session does: one more open
/// descriptor on the same lock file, taken with the same non-blocking `flock`
/// a run takes (`chat::workspace_shape_tests` holds one the same way).
fn hold_lease(state_root: &Path, folder: &Path) -> fs::File {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let root = saya_harness::coding::local::LocalRoot::open(folder, state_root)
        .expect("the other folder is an in-place root");
    let directory = state_root.join("inplace-leases");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{}.lock", root.id().as_str()));
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&path)
        .expect("the lease file opens");
    // SAFETY: `file` owns a valid descriptor for the duration of the call.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(locked, 0, "the test takes its own lease");
    file
}
