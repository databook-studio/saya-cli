//! Contract tests for [`RunPlugin`] and [`PluginRegistry`], driven by a fake
//! plugin whose every contribution the test sets.

#[path = "tests/support.rs"]
mod support;

#[path = "tests/declared_tools.rs"]
mod declared_tools;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use support::{CHAT, FakePlugin, answered, block, labels};

/// Case 1 — an inactive plugin contributes nothing: no definitions, no route,
/// no context (zero bytes), no policy, and no hook calls.
#[tokio::test]
async fn an_inactive_plugin_contributes_nothing() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let plugin = FakePlugin::new("db", false)
        .tools(&["db_query"])
        .serving("db")
        .blocks(vec![block("db-recall", 10)])
        .policy(TurnPolicy {
            max_requests: Some(3),
            max_elapsed: None,
        })
        .logging(calls.clone());
    let registry = PluginRegistry::new(vec![Arc::new(plugin)], &["shell"]).expect("no collision");
    let active = registry.active(&CHAT);

    assert!(active.definitions().is_empty(), "no definitions");
    assert!(active.route("db_query").is_none(), "no route");
    let kept = active.context("prompt", 1024).await;
    assert!(kept.is_empty(), "no context, so the budget stays untouched");
    assert_eq!(active.turn_policy(), None, "no policy");

    active.on_turn_start().await;
    active.on_turn_finish(&answered()).await;
    assert!(
        calls.lock().unwrap().is_empty(),
        "an inactive plugin's hooks never run"
    );
}

/// Case 2 — definitions keep registration order across plugins, so the
/// appended tool list is the same on every run.
#[test]
fn definitions_keep_registration_order() {
    let first = FakePlugin::new("first", true).tools(&["first_one", "first_two"]);
    let second = FakePlugin::new("second", true).tools(&["second_one"]);
    let registry = PluginRegistry::new(vec![Arc::new(first), Arc::new(second)], &["shell"])
        .expect("distinct names");
    let active = registry.active(&CHAT);

    let names: Vec<String> = active
        .definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    assert_eq!(names, vec!["first_one", "first_two", "second_one"]);
}

/// Case 3a — a plugin tool named like a core tool is refused.
#[test]
fn a_tool_named_like_a_core_tool_is_refused() {
    let plugin = FakePlugin::new("db", true).tools(&["shell"]);
    let error = PluginRegistry::new(vec![Arc::new(plugin)], &["shell", "read_file"])
        .expect_err("the core owns shell");
    assert_eq!(
        error,
        PluginError::NameCollision {
            plugin: "db",
            tool: "shell".to_string(),
        }
    );
}

/// Case 3b — two plugins claiming the same tool is refused.
#[test]
fn two_plugins_with_the_same_tool_are_refused() {
    let first = FakePlugin::new("first", true).tools(&["db_query"]);
    let second = FakePlugin::new("second", true).tools(&["db_query"]);
    let error = PluginRegistry::new(vec![Arc::new(first), Arc::new(second)], &["shell"])
        .expect_err("the first plugin owns db_query");
    assert_eq!(
        error,
        PluginError::NameCollision {
            plugin: "second",
            tool: "db_query".to_string(),
        }
    );
}

/// Case 4 — route hands back the owning plugin's executor, and nothing for a
/// name no active plugin declares.
#[tokio::test]
async fn route_returns_the_owning_plugins_executor() {
    let first = FakePlugin::new("first", true)
        .tools(&["first_tool"])
        .serving("first");
    let second = FakePlugin::new("second", true)
        .tools(&["second_tool"])
        .serving("second");
    let registry =
        PluginRegistry::new(vec![Arc::new(first), Arc::new(second)], &[]).expect("distinct");
    let active = registry.active(&CHAT);

    let executor = active
        .route("second_tool")
        .expect("second owns second_tool");
    let result = executor
        .execute("second_tool", serde_json::json!({}))
        .await
        .expect("the fake executor succeeds");
    assert_eq!(result, serde_json::json!({ "executor": "second" }));
    assert!(
        active.route("no_such_tool").is_none(),
        "unknown name routes nowhere"
    );
}

/// Case 5a — a block that would exceed the remaining budget is dropped whole:
/// two blocks of 10 and 20 bytes fit 35, the third 30-byte block does not.
#[tokio::test]
async fn context_drops_a_block_that_would_exceed_the_budget() {
    let plugin = FakePlugin::new("db", true).blocks(vec![
        block("first", 10),
        block("second", 20),
        block("third", 30),
    ]);
    let registry = PluginRegistry::new(vec![Arc::new(plugin)], &[]).expect("no collision");
    let active = registry.active(&CHAT);

    let kept = active.context("prompt", 35).await;
    assert_eq!(labels(&kept), vec!["first", "second"]);
    assert_eq!(
        kept.iter().map(|block| block.body.len()).sum::<usize>(),
        30,
        "no block is truncated mid-body"
    );
}

/// Case 5b — a budget smaller than every block keeps nothing.
#[tokio::test]
async fn context_keeps_nothing_when_the_budget_is_too_small() {
    let plugin = FakePlugin::new("db", true).blocks(vec![block("first", 10), block("second", 20)]);
    let registry = PluginRegistry::new(vec![Arc::new(plugin)], &[]).expect("no collision");
    let active = registry.active(&CHAT);

    assert!(active.context("prompt", 5).await.is_empty());
}

/// Case 6a — the policy is the tightest value of each field across the active
/// plugins; a plugin asking for nothing constrains nothing.
#[test]
fn turn_policy_takes_the_tightest_field() {
    let generous = FakePlugin::new("generous", true).policy(TurnPolicy {
        max_requests: Some(6),
        max_elapsed: Some(Duration::from_secs(90)),
    });
    let strict = FakePlugin::new("strict", true).policy(TurnPolicy {
        max_requests: Some(4),
        max_elapsed: None,
    });
    let silent = FakePlugin::new("silent", true);
    let registry = PluginRegistry::new(
        vec![Arc::new(generous), Arc::new(strict), Arc::new(silent)],
        &[],
    )
    .expect("distinct");
    let active = registry.active(&CHAT);

    assert_eq!(
        active.turn_policy(),
        Some(TurnPolicy {
            max_requests: Some(4),
            max_elapsed: Some(Duration::from_secs(90)),
        })
    );
}

/// Case 6b — no active plugin asking for a policy means no policy at all;
/// absence is not a zero-request turn.
#[test]
fn turn_policy_is_none_when_no_plugin_sets_one() {
    let silent = FakePlugin::new("silent", true);
    let inactive = FakePlugin::new("inactive", false).policy(TurnPolicy {
        max_requests: Some(1),
        max_elapsed: None,
    });
    let registry =
        PluginRegistry::new(vec![Arc::new(silent), Arc::new(inactive)], &[]).expect("distinct");

    assert_eq!(registry.active(&CHAT).turn_policy(), None);
}

/// Case 7 — each hook runs exactly once per active plugin, in registration
/// order, and never on an inactive plugin.
#[tokio::test]
async fn hooks_run_once_per_active_plugin_in_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let first = FakePlugin::new("first", true).logging(calls.clone());
    let skipped = FakePlugin::new("skipped", false).logging(calls.clone());
    let third = FakePlugin::new("third", true).logging(calls.clone());
    let registry = PluginRegistry::new(
        vec![Arc::new(first), Arc::new(skipped), Arc::new(third)],
        &[],
    )
    .expect("distinct");
    let active = registry.active(&CHAT);

    active.on_turn_start().await;
    assert_eq!(*calls.lock().unwrap(), vec!["first:start", "third:start"]);

    calls.lock().unwrap().clear();
    active.on_turn_finish(&answered()).await;
    assert_eq!(*calls.lock().unwrap(), vec!["first:finish", "third:finish"]);
}
