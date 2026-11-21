//! The database budget per surface: `[limits.database]` on a chat turn, the ask
//! surface's override on an ask turn, nothing on a job surface — and no
//! every-turn policy anywhere.
//!
//! The budget is the one the core arms on the turn's FIRST database use
//! (`policy_on_use`), so `turn_policy` states none on every surface.

use std::num::NonZeroU64;
use std::time::Duration;

use saya_agent::{RunContext, RunPlugin, RunSurface, TurnPolicy};

use super::DatabaseBudget;
use super::tests::{chat_budget, chat_plugin_over};

/// A scratch directory under the workspace's own `target/` — never the system
/// temp directory.
fn scratch() -> std::path::PathBuf {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join("tmp-plugins-database-budget");
    std::fs::create_dir_all(&root).expect("scratch directory");
    root
}

/// The resolved config with a `[limits.database]` row and a `[limits.ask]` row
/// of the test's own. Every dimension of the two differs, so a policy that
/// reads the wrong table cannot pass by accident.
fn stated_runtime() -> crate::config::runtime::RuntimeConfig {
    let root = scratch();
    let mut runtime = crate::load_with_sources(
        &crate::GlobalOptions::default(),
        &root,
        &root,
        std::collections::BTreeMap::new(),
    )
    .expect("a runtime resolves from an empty configuration");
    runtime.resolved.limits.database = saya_config::ResolvedAnsweringLimits {
        requests: finite(2),
        tool_calls: finite(3),
        elapsed_seconds: finite(7),
        known_tokens: finite(11),
    };
    runtime.resolved.limits.ask_override = saya_config::ResolvedAnsweringLimits {
        requests: finite(5),
        tool_calls: finite(6),
        elapsed_seconds: finite(9),
        known_tokens: finite(13),
    };
    runtime
}

fn finite(value: u64) -> saya_config::BudgetLimit {
    saya_config::BudgetLimit::Finite(NonZeroU64::new(value).expect("a stated ceiling is nonzero"))
}

fn context(surface: RunSurface) -> RunContext {
    RunContext {
        surface,
        database_profiles: 1,
    }
}

/// Test 3a — a chat turn's budget is all four dimensions of `[limits.database]`.
#[test]
fn a_chat_turn_arms_the_database_budget_on_use() {
    let plugin = chat_plugin_over(DatabaseBudget::of(&stated_runtime()));

    assert_eq!(
        plugin.policy_on_use(&context(RunSurface::Chat)),
        Some(TurnPolicy {
            max_requests: Some(2),
            max_tool_calls: Some(3),
            max_known_tokens: Some(11),
            max_elapsed: Some(Duration::from_secs(7)),
        }),
        "the database budget, from [limits.database]"
    );
}

/// Test 3b — the ask surface's budget is `[limits.ask]`, its unset keys already
/// fallen back to the database table at resolve time.
#[test]
fn an_ask_turn_arms_the_ask_override_on_use() {
    let plugin = chat_plugin_over(DatabaseBudget::ask(&stated_runtime()));

    assert_eq!(
        plugin.policy_on_use(&context(RunSurface::Chat)),
        Some(TurnPolicy {
            max_requests: Some(5),
            max_tool_calls: Some(6),
            max_known_tokens: Some(13),
            max_elapsed: Some(Duration::from_secs(9)),
        }),
        "the ask surface's override, from [limits.ask]"
    );
}

/// Test 3c — no job surface is armed for a budget, whether the plugin was built
/// for the chat or the ask surface: a job keeps its progress guards and its
/// `[limits.job]` backstops.
#[test]
fn a_job_surface_arms_no_database_budget() {
    let chat = chat_plugin_over(DatabaseBudget::of(&stated_runtime()));
    let ask = chat_plugin_over(DatabaseBudget::ask(&stated_runtime()));
    for surface in [
        RunSurface::Headless,
        RunSurface::Background,
        RunSurface::Helper,
    ] {
        assert_eq!(
            chat.policy_on_use(&context(surface)),
            None,
            "a {surface:?} run is not a database question: it keeps its own bounds"
        );
        assert_eq!(ask.policy_on_use(&context(surface)), None, "{surface:?}");
    }
}

/// Test 3d — no surface states an every-turn policy: the database budget is a
/// budget on USE, and a turn that never uses the database spends none of it.
#[test]
fn no_surface_states_an_every_turn_policy() {
    let chat = chat_plugin_over(DatabaseBudget::of(&stated_runtime()));
    let ask = chat_plugin_over(DatabaseBudget::ask(&stated_runtime()));
    for surface in [
        RunSurface::Chat,
        RunSurface::Headless,
        RunSurface::Background,
        RunSurface::Helper,
    ] {
        assert_eq!(chat.turn_policy(&context(surface)), None, "{surface:?}");
        assert_eq!(ask.turn_policy(&context(surface)), None, "{surface:?}");
    }
}

/// Test 3e — an unlimited dimension is armed as absent: absence, not a
/// zero-request turn.
#[test]
fn an_unlimited_dimension_is_armed_as_absent() {
    let mut runtime = stated_runtime();
    runtime.resolved.limits.database.requests = saya_config::BudgetLimit::Unlimited;
    let plugin = chat_plugin_over(DatabaseBudget::of(&runtime));

    let policy = plugin
        .policy_on_use(&context(RunSurface::Chat))
        .expect("the chat still spends its other dimensions");
    assert_eq!(policy.max_requests, None, "unlimited is no ceiling");
    assert_eq!(policy.max_tool_calls, Some(3), "the others are unchanged");
}

/// The product defaults a chat runs under with nothing configured are the
/// database table's own: 24 requests, 64 tool calls, 250 000 known tokens, 300 s.
#[test]
fn an_unconfigured_chat_arms_the_default_database_budget() {
    let plugin = chat_plugin_over(chat_budget());

    assert_eq!(
        plugin.policy_on_use(&context(RunSurface::Chat)),
        Some(TurnPolicy {
            max_requests: Some(24),
            max_tool_calls: Some(64),
            max_known_tokens: Some(250_000),
            max_elapsed: Some(Duration::from_secs(300)),
        }),
        "the defaults a chat's database work has always been bounded by"
    );
}
