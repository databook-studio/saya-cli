//! The one decision a hook makes: whether the call it was watching proceeds.
//!
//! Exit `2` refuses it and hands the model the hook's own words; every other
//! exit lets it through. Nothing here can widen a surface — a hook that allows
//! a call the surface beneath it refuses changes nothing, and a mode that keeps
//! no host lane composes no engine at all.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use saya_agent::{ApprovalPolicy, ToolError, ToolExecutor};
use serde_json::json;

use super::super::gate::HookGate;
use super::super::runner::permits;
use super::{Inner, TempRoot, engine};

/// A `PreToolUse` hook that exits 2 blocks the call, and the reason the model
/// reads is the hook's own stderr.
#[tokio::test]
async fn a_pre_tool_use_hook_blocks_a_named_tool_with_its_own_reason() {
    let root = TempRoot::new("block");
    let engine = engine(
        "[[hooks.pre_tool_use]]\nmatcher = \"write\"\ncommand = \"echo 'no writes today' >&2; exit 2\"\n",
        &root,
    );
    let (inner, ran) = Inner::new();
    let gate = HookGate::new(Arc::clone(&engine), inner);

    let error = gate
        .execute("write", json!({"path": "a.txt"}))
        .await
        .expect_err("the hook must block this call");
    assert_eq!(
        ran.load(Ordering::Relaxed),
        0,
        "a blocked call must never reach the surface"
    );
    let message = error.to_string();
    assert!(
        message.contains("blocked by a PreToolUse hook") && message.contains("no writes today"),
        "the model must read the hook's own reason: {message}"
    );
    assert!(
        error.proves_no_effect(),
        "a call refused before it ran changed nothing"
    );
}

/// Exit 0 allows the call: the surface runs, and the result is the surface's.
#[tokio::test]
async fn a_pre_tool_use_hook_that_exits_zero_allows_the_call() {
    let root = TempRoot::new("allow");
    let engine = engine(
        "[[hooks.pre_tool_use]]\nmatcher = \"read\"\ncommand = \"exit 0\"\n",
        &root,
    );
    let (inner, ran) = Inner::new();
    let gate = HookGate::new(Arc::clone(&engine), inner);

    let result = gate
        .execute("read", json!({"path": "a.txt"}))
        .await
        .expect("an allowing hook must not refuse the call");
    assert_eq!(result, json!({"ok": true}));
    assert_eq!(ran.load(Ordering::Relaxed), 1, "the surface must run");
}

/// A hook that exits 0 on a name the surface does not serve changes nothing:
/// the surface still refuses it. A hook can narrow a surface; it can never
/// widen one.
#[tokio::test]
async fn a_hook_cannot_unblock_a_tool_the_surface_removed() {
    let root = TempRoot::new("unblock");
    let engine = engine("[[hooks.pre_tool_use]]\ncommand = \"exit 0\"\n", &root);
    let (_, ran) = Inner::new();
    let gate = HookGate::new(Arc::clone(&engine), Inner::refusing(ran));

    let error = gate
        .execute("write", json!({"path": "a.txt"}))
        .await
        .expect_err("the surface's own refusal must stand");
    assert!(
        matches!(error, ToolError::UnsupportedTool),
        "the surface's refusal is the one that reaches the model: {error:?}"
    );
}

/// A hook whose matcher names another tool never runs for this call.
#[tokio::test]
async fn a_hook_only_runs_for_the_tool_it_watches() {
    let root = TempRoot::new("matcher");
    let seen = root.path().join("seen");
    let engine = engine(
        &format!(
            "[[hooks.pre_tool_use]]\nmatcher = \"shell\"\ncommand = \"echo ran >> '{}'\"\n",
            seen.display()
        ),
        &root,
    );
    let (inner, ran) = Inner::new();
    let gate = HookGate::new(Arc::clone(&engine), inner);

    gate.execute("read", json!({}))
        .await
        .expect("the call runs");
    assert!(
        !seen.exists(),
        "a hook watching `shell` must not run for `read`"
    );
    assert_eq!(ran.load(Ordering::Relaxed), 1);
}

/// The two modes that keep no host lane compose no engine: a read-only session
/// runs no hook, so a hook cannot be a way around read-only.
#[test]
fn no_engine_is_composed_where_the_mode_keeps_no_host_lane() {
    assert!(permits(ApprovalPolicy::Ask));
    assert!(permits(ApprovalPolicy::Bypass));
    assert!(!permits(ApprovalPolicy::ReadOnly));
    assert!(!permits(ApprovalPolicy::Never));
}

/// The gate leaves the surface's own loop-facing facts alone: the advertised
/// list, the turn note, and settlement all come from the executor beneath it.
#[tokio::test]
async fn the_gate_forwards_the_surfaces_loop_facing_facts() {
    let root = TempRoot::new("forward");
    let engine = engine("[[hooks.stop]]\ncommand = \"true\"\n", &root);
    let (inner, _) = Inner::new();
    let gate = HookGate::new(engine, inner);
    assert_eq!(gate.turn_context_note(), None);
    assert!(gate.advertised_definitions().is_none());
    gate.settle_invocation()
        .await
        .expect("settlement forwards to the surface");
}
