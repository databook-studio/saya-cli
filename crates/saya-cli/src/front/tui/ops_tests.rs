//! One behaviour test per group the surface's table performs: a read, a
//! session op, and the rollovers the chat layer states.
//!
//! Every test reads the surface's own answer — the words the operator would see
//! in the transcript — or the session fact an operation moved. Nothing here
//! drives the run: the table answers on the surface's side, and the fixture's
//! port proves it by recording every call it never received.

use super::ops::Asked;
use super::ops::perform;
use super::ops_fixture::{Fixture, command};
use super::ops_work;
use crate::chat::{ContextChange, RolloverRequest};
use crate::interactive::session::commands::UNDO_NEEDS_A_RUN;

/// The words one command answers with, in the transcript: the table's own, or
/// the shared operation it names, performed the way the port performs it.
fn answer(fixture: &Fixture, line: &str) -> String {
    let asked = perform(&fixture.deps, command(line));
    let asked = match asked {
        Asked::Work(work) => ops_work::perform(&fixture.deps, work),
        asked => asked,
    };
    match asked {
        Asked::Notice(text) | Asked::Error(text) => text,
        other => panic!("`{line}` answers the operator with words, not {other:?}"),
    }
}

/// The read group: `/status` is the one panel, five sections, and it lands in
/// the transcript as the text the line front prints.
#[test]
fn a_status_read_lands_as_the_panel() {
    let fixture = Fixture::new("status");
    let text = answer(&fixture, "/status");
    for heading in ["Session", "Model", "Workspace", "Instructions", "Limits"] {
        assert!(
            text.contains(heading),
            "the panel's `{heading}` section is in the transcript: {text}"
        );
    }
}

/// A bare context read answers with the value in force, in the session-command
/// layer's own words — the same sentence the line front prints.
#[test]
fn a_bare_provider_read_answers_with_the_provider_in_force() {
    let fixture = Fixture::new("provider");
    let in_force = fixture.state().provider.clone();
    let text = answer(&fixture, "/provider");
    assert!(
        text.contains(&format!("Provider: {in_force}")),
        "the read names the provider in force: {text}"
    );
}

/// `/doctor` is a read of the configuration in force, through the one summary
/// the headless `saya config doctor` prints.
#[test]
fn a_doctor_read_lands_as_the_configuration_summary() {
    let fixture = Fixture::new("doctor");
    let text = answer(&fixture, "/doctor");
    assert!(
        !text.trim().is_empty(),
        "the doctor read says something: {text:?}"
    );
    assert!(
        text.to_lowercase().contains("provider") || text.to_lowercase().contains("config"),
        "the summary reads as a configuration check: {text}"
    );
}

/// `/notes` is a read of the session's own record, performed by the state layer
/// that holds it.
#[test]
fn a_notes_read_answers_from_the_session() {
    let fixture = Fixture::new("notes");
    let text = answer(&fixture, "/notes");
    assert!(
        !text.trim().is_empty(),
        "the notes read says something: {text:?}"
    );
}

/// The session-op group: `/thinking` toggles a session fact, and says so.
#[test]
fn a_thinking_toggle_moves_the_session_fact() {
    let fixture = Fixture::new("thinking");
    let text = answer(&fixture, "/thinking");
    assert!(
        text.contains("Thinking display: on"),
        "the toggle says what it did: {text}"
    );
    assert!(
        fixture.state().show_thinking,
        "the session's own fact moved with it"
    );
}

/// `/undo` names the store that does exist here, in the words the shared
/// session-command layer states: this session runs no job, so the run's
/// checkpoints are not its to restore.
#[test]
fn an_undo_names_the_store_this_session_does_have() {
    let fixture = Fixture::new("undo");
    let text = answer(&fixture, "/undo");
    assert!(
        text.contains(UNDO_NEEDS_A_RUN),
        "the undo answer is the shared sentence: {text}"
    );
}

/// A context change is the session-command layer's, exactly as on the line
/// front: the profile moves, the surface says so, and the change is stated as
/// the rollover the port performs on the live run.
#[test]
fn a_connect_moves_the_session_profile() {
    let fixture = Fixture::new("connect");
    match perform(&fixture.deps, command("/connect local")) {
        Asked::Roll { request, said } => {
            assert_eq!(request.change, ContextChange::Connect);
            assert!(
                said.contains("Selected profile: local"),
                "the connect names the profile it selected: {said}"
            );
        }
        other => panic!("`/connect local` states a rollover, not {other:?}"),
    }
    assert_eq!(
        fixture.state().profile.as_deref(),
        Some("local"),
        "the session's own profile moved with it"
    );
}

/// The rollover group: `/clear` is the chat layer's clear, stated as the
/// rollover the session performs — never a word about another surface.
#[test]
fn a_clear_states_the_chat_layers_clear_rollover() {
    let fixture = Fixture::new("clear");
    match perform(&fixture.deps, command("/clear")) {
        Asked::Roll { request, .. } => assert_eq!(
            request,
            RolloverRequest {
                change: ContextChange::Clear,
                value: None,
            },
            "`/clear` is the chat layer's clear"
        ),
        other => panic!("`/clear` states a rollover, not {other:?}"),
    }
    assert!(
        fixture.state().messages.is_empty(),
        "the session's conversation is forgotten with it"
    );
}

/// `/compact` shrinks working memory and `/rotate` continues in a successor:
/// both are the chat layer's rollover, stated once.
#[test]
fn the_compactions_state_their_rollovers() {
    let fixture = Fixture::new("compact");
    for (line, change) in [
        ("/compact", ContextChange::Compact),
        ("/rotate", ContextChange::Rotate),
    ] {
        match perform(&fixture.deps, command(line)) {
            Asked::Roll { request, .. } => assert_eq!(
                request.change, change,
                "`{line}` is the chat layer's own rollover"
            ),
            other => panic!("`{line}` states a rollover, not {other:?}"),
        }
    }
}

/// The table never enters the run: every operation above stayed on the
/// surface's side of the port.
#[test]
fn no_operation_enters_the_run() {
    let fixture = Fixture::new("port");
    for line in ["/status", "/undo", "/clear", "/thinking", "/connect local"] {
        let _ = perform(&fixture.deps, command(line));
    }
    assert_eq!(
        fixture.port.calls(),
        Vec::<&'static str>::new(),
        "the table performs every operation outside the run"
    );
}
