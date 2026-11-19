//! The job's one permission-mode cell at the host: a headless composition's
//! mode is frozen, and a host two `Arc`s hold answers with the mode a setter
//! stated rather than passing silently over it.
#![cfg(unix)]

use super::{CodingApplication, Launch};
use crate::{GlobalOptions, load_with_sources};
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

/// A scratch folder to open over, its state directory, and a program directory
/// the host accepts.
struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    programs: PathBuf,
    git: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(case: &str) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-coding-mode-{case}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&root).expect("the fixture root is creatable");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .expect("the fixture root is private");
    let root = root.canonicalize().expect("the fixture root resolves");
    let repository = root.join("folder");
    fs::create_dir(&repository).expect("the folder is creatable");
    let programs = root.join("programs");
    fs::create_dir(&programs).expect("the program directory is creatable");
    let git = super::cli::find_git().expect("git is on PATH");
    Fixture {
        root,
        repository,
        programs,
        git,
    }
}

async fn open(fixture: &Fixture) -> Arc<CodingApplication> {
    let runtime = load_with_sources(
        &GlobalOptions::default(),
        &fixture.repository,
        &fixture.root,
        BTreeMap::new(),
    )
    .expect("the runtime config loads");
    CodingApplication::open_with_mode(
        runtime,
        SqliteStateStore::new(fixture.root.join("state.sqlite3")),
        &fixture.repository,
        &fixture.root.join("state"),
        &fixture.git,
        &fixture.programs,
        crate::cli::RootMode::InPlace,
        &CancellationToken::new(),
    )
    .await
    .expect("the in-place application opens")
}

/// A headless run's approval is stated before it starts: `saya -p` freezes its
/// mode at composition, so a later change is refused rather than applied.
#[tokio::test]
async fn a_headless_composition_refuses_a_mode_change() {
    let fixture = fixture("headless");
    let application = open(&fixture)
        .await
        .with_frozen_approval_policy(ApprovalPolicy::ReadOnly);

    assert_eq!(application.mode.get(), ApprovalPolicy::ReadOnly);
    assert!(
        application
            .set_approval_policy(ApprovalPolicy::Bypass)
            .is_err(),
        "a frozen job refuses a mode change"
    );
    assert_eq!(
        application.mode.get(),
        ApprovalPolicy::ReadOnly,
        "and its mode is unchanged"
    );
}

/// The old silent no-op: `with_approval_policy` mutated only through
/// `Arc::get_mut`, so a host two `Arc`s held kept its old mode without saying
/// so. A setter now either lands or is refused as frozen — never ignored.
#[tokio::test]
async fn a_shared_host_answers_with_the_mode_a_setter_stated() {
    let fixture = fixture("shared");
    let application = open(&fixture).await;
    let held_elsewhere = Arc::clone(&application);

    assert!(
        held_elsewhere
            .set_approval_policy(ApprovalPolicy::ReadOnly)
            .is_ok(),
        "an unfrozen job takes the change"
    );
    assert_eq!(
        application.mode.get(),
        ApprovalPolicy::ReadOnly,
        "the host both `Arc`s hold reads the stated mode"
    );

    let frozen = open(&fixture)
        .await
        .with_frozen_approval_policy(ApprovalPolicy::Ask);
    let shared = Arc::clone(&frozen);
    assert!(
        shared.set_approval_policy(ApprovalPolicy::Bypass).is_err(),
        "a frozen job refuses rather than pretending"
    );
    assert_eq!(frozen.mode.get(), ApprovalPolicy::Ask);
}

/// `saya start` runs detached from every operator surface, so its composed
/// mode is frozen: the mode a later call reads is the one the launch stated.
#[tokio::test]
async fn a_detached_start_refuses_a_mode_change() {
    let fixture = fixture("detached-start");
    let application = Launch::Detached(ApprovalPolicy::Ask).compose(open(&fixture).await);

    assert!(
        application
            .set_approval_policy(ApprovalPolicy::ReadOnly)
            .is_err(),
        "a detached start refuses a mode change"
    );
    assert_eq!(
        application.mode.get(),
        ApprovalPolicy::Ask,
        "and its mode is the one the launch stated"
    );
}

/// `saya follow-up` resumes a job in the background: the same detachment, so
/// the same refusal.
#[tokio::test]
async fn a_detached_follow_up_refuses_a_mode_change() {
    let fixture = fixture("detached-follow-up");
    let application = Launch::Detached(ApprovalPolicy::ReadOnly).compose(open(&fixture).await);

    assert!(
        application
            .set_approval_policy(ApprovalPolicy::Bypass)
            .is_err(),
        "a detached follow-up refuses a mode change"
    );
    assert_eq!(
        application.mode.get(),
        ApprovalPolicy::ReadOnly,
        "and its mode is the one the launch stated"
    );
}

/// `saya job resume` is composed by the same detached launch as `saya start`
/// and `saya follow-up`; it freezes there too.
#[tokio::test]
async fn a_detached_job_resume_refuses_a_mode_change() {
    let fixture = fixture("detached-resume");
    let application = Launch::Detached(ApprovalPolicy::Bypass).compose(open(&fixture).await);

    assert!(
        application
            .set_approval_policy(ApprovalPolicy::Ask)
            .is_err(),
        "a detached resume refuses a mode change"
    );
    assert_eq!(
        application.mode.get(),
        ApprovalPolicy::Bypass,
        "and its mode is the one the launch stated"
    );
}

/// A start that falls back to an in-place root reopens a fresh host; the
/// fallback must not hand back one whose mode a later call could move.
#[tokio::test]
async fn a_detached_fallback_stays_frozen() {
    let fixture = fixture("detached-fallback");
    let application = Launch::Detached(ApprovalPolicy::ReadOnly).compose(open(&fixture).await);
    let reopened = application.reopen_mode(open(&fixture).await);

    assert!(
        reopened
            .set_approval_policy(ApprovalPolicy::Bypass)
            .is_err(),
        "the reopened in-place host is frozen too"
    );
    assert_eq!(
        reopened.mode.get(),
        ApprovalPolicy::ReadOnly,
        "and it kept the mode the launch stated"
    );
}

/// The interactive session states a mode its operator can still switch, so an
/// attached composition takes the change.
#[tokio::test]
async fn an_attached_session_takes_a_mode_change() {
    let fixture = fixture("attached");
    let application = Launch::Attached(ApprovalPolicy::Ask).compose(open(&fixture).await);

    assert!(
        application
            .set_approval_policy(ApprovalPolicy::ReadOnly)
            .is_ok(),
        "an attached session takes the change"
    );
    assert_eq!(application.mode.get(), ApprovalPolicy::ReadOnly);
}
