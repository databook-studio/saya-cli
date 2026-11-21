//! The workspace trust question (`02` §9.4): the runtime opens it when the
//! session bound no workspace, `y` binds the launch folder through today's
//! store, and `n` continues without one.
//!
//! The binding is asserted twice over: at the store, whose recording double
//! says exactly which folder an answer binds, and at the port, which is what the
//! surface tells the run.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::{Mutex, PoisonError};

use ratatui::crossterm::event::KeyCode;
use tokio::sync::mpsc;

use super::direct::DirectOutcome;
use super::direct_fixture::Fixture;
use super::direct_trust::{self, BODY};
use super::msg::{Cmd, Msg};
use super::run::{FRAME, Runtime};
use super::run_cmd;
use super::run_draw::Facts;
use super::run_fixture::{FakeScreen, Recorded, key};
use super::run_fixture_port::{Call, FakeClock, FakePort};
use super::update::update;
use crate::interactive::tui::TrustResolution;

/// The launch folder, resolved the way today's answer resolves it.
fn launch_folder() -> PathBuf {
    crate::interactive::session::permissions::trust::resolve_trusted_dir(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    )
    .expect("the launch folder resolves")
}

/// The last frame the fake screen painted.
fn last(recorded: &Arc<Mutex<Recorded>>) -> String {
    recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .last_frame()
        .to_owned()
}

#[test]
fn y_binds_the_launch_folder_through_the_store() {
    let fixture = Fixture::new("trust-yes");
    let worker = direct_trust::answer(fixture.trust.as_ref(), TrustResolution::TrustCwd);

    assert_eq!(
        *fixture.trust.bound(),
        vec![launch_folder()],
        "`y` binds exactly the launch folder, canonicalised"
    );
    assert!(
        worker.message.is_empty(),
        "a bound folder is filed in the transcript, not toasted"
    );
    assert!(
        matches!(worker.direct, Some(DirectOutcome::Notice(_))),
        "the trust echo is the surface's own line: {:?}",
        worker.direct
    );
}

#[test]
fn n_binds_nothing_and_the_surface_states_no_command() {
    let fixture = Fixture::new("trust-no");
    let mut model = fixture.model();
    model.open_trust();

    let cmds = update(&mut model, key(KeyCode::Char('n')));
    assert!(
        cmds.is_empty(),
        "`n` continues unbound: nothing to perform, got {cmds:?}"
    );
    assert!(!model.overlay_open(), "the question is closed");
    assert!(
        fixture.trust.bound().is_empty(),
        "the store was never asked to bind anything"
    );
}

#[test]
fn y_on_the_open_question_states_one_trust_command() {
    let fixture = Fixture::new("trust-accept");
    let mut model = fixture.model();
    model.open_trust();
    let port = FakePort::new();

    let cmds = update(&mut model, key(KeyCode::Char('y')));
    assert_eq!(cmds, vec![Cmd::Trust], "`y` answers with one trust command");
    for cmd in cmds {
        run_cmd::perform(&port, cmd);
    }
    assert_eq!(
        port.calls(),
        vec![Call::Trust(TrustResolution::TrustCwd)],
        "the port carries the answer out"
    );
    assert!(
        !model.overlay_open(),
        "the question is closed once answered"
    );
}

#[tokio::test]
async fn the_runtime_opens_the_question_and_y_binds_through_the_port() {
    let (sender, queue) = mpsc::unbounded_channel();
    let screen = FakeScreen::new(100, 30);
    let recorded = screen.recorded();
    let clock = FakeClock::new();
    let port = Arc::new(FakePort::new());
    let mut runtime = Runtime::new(
        super::model::Model::default(),
        queue,
        screen,
        clock.clone(),
        Arc::clone(&port),
    )
    .with_facts(Facts {
        untrusted: true,
        ..Facts::default()
    });

    sender
        .send(Msg::Resize {
            width: 100,
            height: 30,
        })
        .expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");
    let painted = last(&recorded);
    assert!(
        painted.contains("trust this folder?"),
        "an untrusted session is asked the question: {painted}"
    );
    assert!(
        painted.contains("trust this folder    [n]"),
        "the answers row names the keys this surface reads: {painted}"
    );
    assert!(
        BODY.starts_with("No workspace is bound"),
        "the question is today's prompt body"
    );

    sender
        .send(key(KeyCode::Char('y')))
        .expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");
    assert_eq!(
        port.calls(),
        vec![Call::Trust(TrustResolution::TrustCwd)],
        "`y` reaches the run as today's trust answer"
    );
    let painted = last(&recorded);
    assert!(
        !painted.contains("trust this folder?"),
        "the question is answered and gone: {painted}"
    );
}
