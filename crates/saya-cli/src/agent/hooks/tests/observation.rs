//! The events that observe, and the bounds that keep them from wedging the
//! loop: `PostToolUse` and `Stop` decide nothing, a hung hook is killed at its
//! own timeout, and the payload a hook reads is redacted and bounded.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use saya_agent::ToolExecutor;
use saya_config::HookEvent;
use serde_json::json;

use super::super::gate::HookGate;
use super::{Inner, TempRoot, engine, hooks_of};

/// A `PostToolUse` hook sees the call it followed, and its exit code changes
/// nothing: the result the model reads is the surface's own.
#[tokio::test]
async fn a_post_tool_use_hook_sees_the_call() {
    let root = TempRoot::new("post");
    let seen = root.path().join("seen");
    let engine = engine(
        &format!(
            "[[hooks.post_tool_use]]\nmatcher = \"read\"\ncommand = \"cat >> '{}'\"\n",
            seen.display()
        ),
        &root,
    );
    let (inner, ran) = Inner::new();
    let gate = HookGate::new(Arc::clone(&engine), inner);

    let result = gate
        .execute("read", json!({"path": "a.txt"}))
        .await
        .expect("the call must run");
    assert_eq!(result, json!({"ok": true}));
    assert_eq!(ran.load(Ordering::Relaxed), 1);
    let recorded = std::fs::read_to_string(&seen).expect("the hook must have written its input");
    assert!(
        recorded.contains("\"hook_event_name\":\"PostToolUse\"")
            && recorded.contains("\"tool_name\":\"read\"")
            && recorded.contains("\"tool_succeeded\":true"),
        "the hook must read the call it followed: {recorded}"
    );
}

/// A tool call that fails still runs its `PostToolUse` hook: the hook watches
/// calls, not successes.
#[tokio::test]
async fn a_failing_call_still_runs_its_post_hook() {
    let root = TempRoot::new("post-fail");
    let seen = root.path().join("seen");
    let engine = engine(
        &format!(
            "[[hooks.post_tool_use]]\ncommand = \"cat >> '{}'\"\n",
            seen.display()
        ),
        &root,
    );
    let (_, ran) = Inner::new();
    let gate = HookGate::new(Arc::clone(&engine), Inner::refusing(ran));

    let _ = gate.execute("write", json!({})).await;
    let recorded = std::fs::read_to_string(&seen).expect("the post hook must have run");
    assert!(
        recorded.contains("\"tool_succeeded\":false"),
        "the hook must see that the call failed: {recorded}"
    );
}

/// The `stop` hooks run when the agent finishes a turn, and see who finished.
#[tokio::test]
async fn the_stop_hook_runs_when_the_turn_ends() {
    let root = TempRoot::new("stop");
    let seen = root.path().join("seen");
    let engine = engine(
        &format!(
            "[[hooks.stop]]\ncommand = \"cat >> '{}'\"\n",
            seen.display()
        ),
        &root,
    );
    engine.stop().await;
    let recorded = std::fs::read_to_string(&seen).expect("the stop hook must have run");
    assert!(
        recorded.contains("\"hook_event_name\":\"Stop\"")
            && recorded.contains("\"job_id\":\"job-1\""),
        "the stop hook must see the event and the job: {recorded}"
    );
}

/// A hook that never finishes is killed at its own timeout, and the call it
/// was watching proceeds: a hung hook must not wedge the agent loop.
#[tokio::test]
async fn a_hook_that_times_out_is_killed_and_does_not_block() {
    let root = TempRoot::new("timeout");
    let engine = engine(
        "[[hooks.pre_tool_use]]\ncommand = \"sleep 30\"\ntimeout_ms = 300\n",
        &root,
    );
    let (inner, ran) = Inner::new();
    let gate = HookGate::new(Arc::clone(&engine), inner);

    let started = Instant::now();
    let result = gate
        .execute("read", json!({}))
        .await
        .expect("a timed-out hook must not block");
    assert_eq!(result, json!({"ok": true}));
    assert_eq!(ran.load(Ordering::Relaxed), 1, "the surface must still run");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the timeout, not the hook's own sleep, must end the wait: {:?}",
        started.elapsed()
    );
}

/// The arguments a hook reads are bounded and redacted: a payload carrying a
/// credential-shaped value does not hand the hook the value, and a payload
/// larger than the bound is cut rather than refused.
#[tokio::test]
async fn the_input_is_redacted_and_bounded() {
    let root = TempRoot::new("payload");
    let seen = root.path().join("seen");
    let engine = engine(
        &format!(
            "[[hooks.pre_tool_use]]\ncommand = \"cat >> '{}'\"\n",
            seen.display()
        ),
        &root,
    );
    let secret = "hunter2-not-for-the-hook";
    let huge = "x".repeat(200 * 1024);
    engine
        .pre_tool_use(
            "shell",
            &json!({"command": huge, "sql": format!("select 1 where password='{secret}'")}),
        )
        .await
        .expect("no hook blocks here");
    let recorded = std::fs::read_to_string(&seen).expect("the hook must have run");
    assert!(
        !recorded.contains(secret),
        "a credential-shaped argument must not reach the hook: {recorded}"
    );
    assert!(
        recorded.len() <= saya_harness::runner::shell::MAX_STDIN_BYTES,
        "the payload must fit the shell's stdin bound: {} bytes",
        recorded.len()
    );
    assert!(
        recorded.contains("elided"),
        "an oversized argument is cut, not dropped: {recorded}"
    );
}

/// The resolved table's own shape: an event with no hook runs nothing, so an
/// engine built from an empty table is never composed at all.
#[test]
fn an_empty_table_holds_no_hook() {
    let hooks = hooks_of("");
    assert!(hooks.is_empty());
    assert!(hooks.for_event(HookEvent::PreToolUse).is_empty());
    assert!(hooks.for_event(HookEvent::PostToolUse).is_empty());
    assert!(hooks.for_event(HookEvent::Stop).is_empty());
}
