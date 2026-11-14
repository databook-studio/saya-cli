//! `SessionStart` and `SessionEnd`: the start context is offered once and is
//! bounded; the end hook runs even after the session's own cancellation fired.

use std::sync::Arc;

use saya_agent::CancellationToken;
use saya_harness::runner::HostPolicy;

use super::super::runner::{HookCaller, Hooks};
use super::super::session::SessionHooks;
use super::{TempRoot, engine, hooks_of};

/// The start hook runs once per session process, and its stdout is the
/// context every turn reads.
#[tokio::test]
async fn the_session_start_hook_runs_once_and_returns_context() {
    let root = TempRoot::new("session-start");
    let seen = root.path().join("seen");
    let engine = engine(
        &format!(
            "[[hooks.session_start]]\ncommand = \"printf 'use the staging database\\n'; echo ran >> '{}'\"\n",
            seen.display()
        ),
        &root,
    );
    let cell = SessionHooks::new();

    let first = cell.start(&engine).await;
    let second = cell.start(&engine).await;
    assert_eq!(first.as_deref(), Some("use the staging database"));
    assert_eq!(second, first, "every turn reads the same context");
    let runs = std::fs::read_to_string(&seen).expect("the hook must have run");
    assert_eq!(
        runs.lines().count(),
        1,
        "the start hook runs once, not once per turn"
    );
}

/// A non-zero exit prints a diagnostic, not context.
#[tokio::test]
async fn a_failing_session_start_offers_no_context() {
    let root = TempRoot::new("session-start-fail");
    let engine = engine(
        "[[hooks.session_start]]\ncommand = \"printf 'noise'; exit 3\"\n",
        &root,
    );
    assert_eq!(SessionHooks::new().start(&engine).await, None);
}

/// The start context is bounded: a hook cannot spend more of the turn than a
/// refusal can.
#[tokio::test]
async fn the_session_start_context_is_bounded() {
    let root = TempRoot::new("session-start-bound");
    let engine = engine(
        "[[hooks.session_start]]\ncommand = \"yes context | head -c 20000\"\n",
        &root,
    );
    let context = SessionHooks::new()
        .start(&engine)
        .await
        .expect("the hook printed context");
    assert!(
        context.len() <= super::super::runner::MAX_SESSION_CONTEXT_BYTES,
        "the context is bounded: {} bytes",
        context.len()
    );
}

/// The start payload names the event and the caller.
#[tokio::test]
async fn the_session_start_payload_names_the_event() {
    let root = TempRoot::new("session-start-payload");
    let seen = root.path().join("seen");
    let engine = engine(
        &format!(
            "[[hooks.session_start]]\ncommand = \"cat >> '{}'\"\n",
            seen.display()
        ),
        &root,
    );
    engine.session_start().await;
    let recorded = std::fs::read_to_string(&seen).expect("the hook must have run");
    assert!(
        recorded.contains("\"hook_event_name\":\"SessionStart\"")
            && recorded.contains("\"job_id\":\"job-1\""),
        "the hook must read the event and the caller: {recorded}"
    );
}

/// The end hook still runs after the session's own cancellation fired, and
/// its payload names the end.
#[tokio::test]
async fn the_session_end_hook_runs_after_cancellation() {
    let root = TempRoot::new("session-end");
    let seen = root.path().join("seen");
    let cancellation = CancellationToken::new();
    let hooks = Arc::new(
        Hooks::new(
            hooks_of(&format!(
                "[[hooks.session_end]]\ncommand = \"cat >> '{}'\"\n",
                seen.display()
            )),
            root.path(),
            HostPolicy::default(),
            HookCaller::Job("job-1".into()),
            cancellation.clone(),
        )
        .expect("a host-only engine constructs over a real root"),
    );
    cancellation.cancel();

    hooks.session_end().await;
    let recorded = std::fs::read_to_string(&seen).expect("the end hook must run after a cancel");
    assert!(
        recorded.contains("\"hook_event_name\":\"SessionEnd\"")
            && recorded.contains("\"job_id\":\"job-1\""),
        "the end hook must read the event and the caller: {recorded}"
    );
}
