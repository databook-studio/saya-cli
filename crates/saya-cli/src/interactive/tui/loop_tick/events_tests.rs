//! The tick that drains a queued permission cycle: the flag the key handler
//! sets becomes a real switch on the session's one policy, and a refusal
//! lands as an Error block that changes nothing.

use super::apply_permission_cycle;
use crate::interactive::session::permissions::mode::name as mode_name;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::application::tests_support::{idle_app, unused_runtime};
use crate::interactive::tui::types::App;
use saya_agent::ApprovalPolicy;

/// A throwaway sessions root, named per test so two parallel tests never
/// share a state directory.
fn root_for(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-tick-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the fixture root");
    root
}

/// One acquired session plus an idle app, with the throwaway root removed on
/// drop.
struct Fixture {
    app: App,
    state: SessionState,
    session: SessionRuntime,
    _root: std::path::PathBuf,
}

impl Fixture {
    fn acquire(tag: &str, allow_bypass: bool) -> Self {
        let root = root_for(tag);
        let sessions = root.join("sessions");
        std::fs::create_dir_all(&sessions).expect("the sessions root");
        let session = SessionRuntime::acquire(
            &unused_runtime(),
            None,
            false,
            None,
            "20260101-000000-tick",
            ApprovalPolicy::Ask,
            &sessions,
        )
        .expect("the fixture session acquires");
        let mut state = SessionState::new("20260101-000000-tick", None, "test-model");
        state.approval_mode = "ask".into();
        state.allow_bypass = allow_bypass;
        Self {
            app: idle_app(),
            state,
            session,
            _root: root,
        }
    }

    /// What the transcript says, joined — the words the user reads.
    fn said(&self) -> String {
        self.app
            .transcript
            .blocks()
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self._root);
    }
}

/// A queued cycle becomes a real switch: the pair moves, the session's one
/// policy moves with it, and the flag is spent — a second tick does nothing.
#[test]
fn a_queued_cycle_switches_the_session_once() {
    let mut fixture = Fixture::acquire("cycle", false);
    fixture.app.pending_permission_cycle = true;
    apply_permission_cycle(&mut fixture.app, &mut fixture.state, &mut fixture.session);

    assert!(
        !fixture.app.pending_permission_cycle,
        "the flag is spent, so the next tick is a no-op"
    );
    assert_eq!(
        (
            fixture.state.approval_mode.as_str(),
            fixture.state.agent_mode.as_str()
        ),
        ("accept-edits", "build"),
        "the first cycle lands on accept-edits"
    );
    assert_eq!(
        fixture.session.policy().mode(),
        ApprovalPolicy::AcceptEdits,
        "the session's one policy moved with the pair"
    );
    assert!(
        fixture.said().contains("Mode: accept-edits"),
        "the switch's own answer lands in the transcript: {}",
        fixture.said()
    );

    apply_permission_cycle(&mut fixture.app, &mut fixture.state, &mut fixture.session);
    assert_eq!(
        fixture.state.approval_mode, "accept-edits",
        "a tick with nothing queued switches nothing"
    );
}

/// A withheld `bypass` refuses at the tick exactly as it does at `/mode`:
/// the cycle skips it, so the walk closes on `ask` — never on a mode the
/// configuration withholds.
#[test]
fn the_cycle_skips_a_withheld_bypass() {
    let mut fixture = Fixture::acquire("cycle-skip", false);
    let mut seen = Vec::new();
    for _ in 0..3 {
        fixture.app.pending_permission_cycle = true;
        apply_permission_cycle(&mut fixture.app, &mut fixture.state, &mut fixture.session);
        let mode = crate::interactive::session::permissions::mode::current(&fixture.state)
            .expect("every reachable member is one of the four modes");
        seen.push(mode_name(mode).to_owned());
    }
    assert_eq!(seen, ["accept-edits", "plan", "ask"]);
    assert!(
        !fixture.said().contains("bypass on:"),
        "no consent is recorded for a mode the cycle never reaches"
    );
}

/// A cycle into `plan` closes the write surface at the engine, not only in
/// the bar's word.
#[test]
fn a_cycle_into_plan_closes_the_write_surface() {
    let mut fixture = Fixture::acquire("cycle-plan", false);
    for _ in 0..2 {
        fixture.app.pending_permission_cycle = true;
        apply_permission_cycle(&mut fixture.app, &mut fixture.state, &mut fixture.session);
    }
    assert_eq!(fixture.state.agent_mode, "plan");
    assert_eq!(
        fixture
            .session
            .policy()
            .resolve(&write_shape(), Some(saya_agent::WORKSPACE_WRITE_TOKEN)),
        saya_agent::ApprovalDecision::Deny { reason: None },
        "plan denies a workspace write before the approval match"
    );
}

/// The `write`/`edit` family's shape: a workspace file write needing an
/// approval and carrying the token the mode's pre-answer names.
fn write_shape() -> saya_agent::ToolEffect {
    saya_agent::ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: true,
        local_state: saya_agent::LocalStateEffect::WriteWorkspace,
    }
}
