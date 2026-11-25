//! What the operations do: which ones move the session, which only ask for a
//! rollover, and which never reach the run at all.

use saya_agent::ApprovalPolicy;

use super::OpResult;
use super::fixture::{command, harness};
use crate::chat::ops_context::{ContextChange, ReadOp, RolloverRequest};
use crate::interactive::session::permissions::mode::BYPASS_REFUSAL;

#[test]
fn model_change_requests_a_rollover_and_never_switches_in_place() {
    let harness = harness(ApprovalPolicy::Ask, false);
    let result = harness.ops.run(command("/model sonnet"));
    assert_eq!(
        result,
        OpResult::Rollover(RolloverRequest {
            change: ContextChange::Model,
            value: Some("sonnet".to_owned()),
        }),
        "a model change is a request for the session to roll over"
    );
    assert!(
        harness.port.calls().is_empty(),
        "a context change is never stated on the running run: {:?}",
        harness.port.calls()
    );
}

#[test]
fn mode_plan_narrows_the_runs_next_call() {
    let harness = harness(ApprovalPolicy::Ask, false);
    let result = harness.ops.run(command("/mode plan"));
    let OpResult::Done(words) = result else {
        panic!("`/mode plan` is a session op: {result:?}");
    };
    assert!(
        words.contains("read-only"),
        "the words name the posture the next call reads: {words}"
    );
    assert_eq!(
        harness.cell.get(),
        ApprovalPolicy::ReadOnly,
        "plan states read-only on the run's cell"
    );
    assert!(
        harness.port.calls().is_empty(),
        "a mode switch is a cell, never a turn: {:?}",
        harness.port.calls()
    );
}

#[test]
fn mode_bypass_without_allow_bypass_is_refused_in_todays_words() {
    let harness = harness(ApprovalPolicy::Ask, false);
    let result = harness.ops.run(command("/mode bypass"));
    assert_eq!(
        result,
        OpResult::Refused(BYPASS_REFUSAL.to_owned()),
        "the refusal is the mode module's own wording"
    );
    assert_eq!(
        harness.cell.get(),
        ApprovalPolicy::Ask,
        "a refused switch changes nothing"
    );
}

#[test]
fn mode_bypass_is_stated_where_the_configuration_allows_it() {
    let harness = harness(ApprovalPolicy::Ask, true);
    let result = harness.ops.run(command("/mode bypass"));
    assert!(
        matches!(result, OpResult::Done(_)),
        "`bypass` is reachable once the configuration allows it: {result:?}"
    );
    assert_eq!(harness.cell.get(), ApprovalPolicy::Bypass);
}

#[test]
fn a_bare_mode_reads_the_runs_cell() {
    let harness = harness(ApprovalPolicy::AcceptEdits, false);
    let result = harness.ops.run(command("/mode"));
    let OpResult::Done(words) = result else {
        panic!("a bare `/mode` reads: {result:?}");
    };
    assert!(
        words.contains("accept-edits"),
        "the words name what the next call reads: {words}"
    );
}

#[test]
fn a_mode_never_widens_a_cell_narrower_than_read_only() {
    let harness = harness(ApprovalPolicy::Never, false);
    let result = harness.ops.run(command("/mode plan"));
    let OpResult::Done(words) = result else {
        panic!("`/mode plan` answers: {result:?}");
    };
    assert_eq!(
        harness.cell.get(),
        ApprovalPolicy::Never,
        "`never` is narrower than plan's read-only, and a switch never widens it"
    );
    assert!(words.contains("never"), "{words}");
}

#[test]
fn status_answers_without_entering_the_run() {
    let harness = harness(ApprovalPolicy::Ask, false);
    // A turn is in flight whenever the port is busy; `/status` is a read, so it
    // is answered the same as any other moment — and the port sees nothing.
    assert_eq!(
        harness.ops.run(command("/status")),
        OpResult::Read(ReadOp::Status)
    );
    assert_eq!(
        harness.ops.run(command("/status")),
        OpResult::Read(ReadOp::Status),
        "a repeated `/status` is the same read"
    );
    assert!(
        harness.port.calls().is_empty(),
        "the panel reads facts the session already holds: {:?}",
        harness.port.calls()
    );
}

#[test]
fn the_status_aliases_open_their_sections() {
    use crate::chat::ops_status::Section;
    let harness = harness(ApprovalPolicy::Ask, false);
    for (text, section) in [
        ("/workspace", Section::Workspace),
        ("/context", Section::Instructions),
    ] {
        assert_eq!(
            harness.ops.run(command(text)),
            OpResult::Read(ReadOp::StatusSection(section)),
            "{text} opens its section of the one panel"
        );
    }
    assert_eq!(
        harness.ops.run(command("/usage")),
        OpResult::Read(ReadOp::Usage(crate::slash::UsageRequest::show())),
        "`/usage` is its own panel now, not a section of `/status`"
    );
    assert!(
        harness.port.calls().is_empty(),
        "no alias reaches the run: {:?}",
        harness.port.calls()
    );
}

#[test]
fn history_reads_the_sessions_listing() {
    let harness = harness(ApprovalPolicy::Ask, false);
    assert_eq!(
        harness.ops.run(command("/history")),
        OpResult::Read(ReadOp::Sessions),
        "`/history` is `/sessions`: one read"
    );
    assert_eq!(
        harness.ops.run(command("/sessions")),
        OpResult::Read(ReadOp::Sessions)
    );
}

#[test]
fn sql_never_touches_the_run() {
    let harness = harness(ApprovalPolicy::Ask, false);
    assert_eq!(
        harness.ops.run(command("/sql select 1")),
        OpResult::Read(ReadOp::Sql("select 1".to_owned()))
    );
    assert!(
        harness.port.calls().is_empty(),
        "`/sql` is a shared read operation, not a turn: {:?}",
        harness.port.calls()
    );
}

#[test]
fn no_read_only_command_enters_the_run() {
    let harness = harness(ApprovalPolicy::Ask, false);
    for text in [
        "/connections",
        "/schema",
        "/sql select 1",
        "/history",
        "/sessions list",
        "/help",
        "/grants",
        "/context",
        "/doctor",
        "/status",
        "/usage",
        "/workspace",
        "/runs",
        "/tasks",
        "/notes",
        "/recovery",
    ] {
        harness.ops.run(command(text));
    }
    assert!(
        harness.port.calls().is_empty(),
        "no read-only command reaches the run: {:?}",
        harness.port.calls()
    );
}

#[test]
fn a_rollover_request_carries_what_changed_and_what_to() {
    let harness = harness(ApprovalPolicy::Ask, false);
    let cases = [
        ("/connect analytics", ContextChange::Connect, "analytics"),
        ("/include warehouse", ContextChange::Include, "warehouse"),
        ("/privacy off", ContextChange::Privacy, "off"),
    ];
    for (text, change, value) in cases {
        assert_eq!(
            harness.ops.run(command(text)),
            OpResult::Rollover(RolloverRequest {
                change,
                value: Some(value.to_owned()),
            }),
            "{text}"
        );
    }
    assert_eq!(
        harness.ops.run(command("/clear")),
        OpResult::Rollover(RolloverRequest {
            change: ContextChange::Clear,
            value: None,
        }),
        "`/clear` rolls over with an empty seed, so it states no value"
    );
}

#[test]
fn a_session_op_is_handed_on_with_its_argument() {
    let harness = harness(ApprovalPolicy::Ask, false);
    assert!(matches!(
        harness.ops.run(command("/allow sql:analytics")),
        OpResult::Session(crate::chat::ops_context::SessionOp::Allow(tokens))
            if tokens == vec!["sql:analytics".to_owned()]
    ));
}

/// A hidden command still runs (K8): `/recovery list` is absent from the
/// listing, the popup, the palette and the typo suggestion, and typing it maps
/// to the same read operation it always did — the hide is presentation only.
#[test]
fn a_hidden_recovery_list_still_maps_to_its_read() {
    use crate::commands::recovery::RecoveryCommand;
    let harness = harness(ApprovalPolicy::Ask, false);
    assert_eq!(
        harness.ops.run(command("/recovery list")),
        OpResult::Read(ReadOp::Recovery(RecoveryCommand::List)),
        "a hidden command is still the operation it was"
    );
    assert!(
        harness.port.calls().is_empty(),
        "a read of the per-edit store never enters the run: {:?}",
        harness.port.calls()
    );
}
