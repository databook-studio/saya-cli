//! The database plugin's turn policy: the chat's question budget, and nothing
//! on a job surface.
//!
//! A chat turn is a database question, so the plugin hands the core all four
//! dimensions of `[limits.session]`; a headless, background or helper run keeps
//! its own bounds and is asked for no policy at all.

use std::num::NonZeroU64;
use std::time::Duration;

use saya_agent::{RunContext, RunPlugin, RunSurface, TurnPolicy};

use super::SessionBudget;
use super::tests::{chat_plugin_over, chat_session};

/// A session ceiling the product's defaults never spell, so a policy that
/// reads them cannot pass by accident.
fn session() -> SessionBudget {
    SessionBudget::of(&session_runtime())
}

/// The resolved config with a `[limits.session]` row of the test's own.
fn session_runtime() -> crate::config::runtime::RuntimeConfig {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join("tmp-plugins-database-policy");
    std::fs::create_dir_all(&root).expect("scratch directory");
    let mut runtime = crate::load_with_sources(
        &crate::GlobalOptions::default(),
        &root,
        &root,
        std::collections::BTreeMap::new(),
    )
    .expect("a runtime resolves from an empty configuration");
    runtime.resolved.limits.session = saya_config::ResolvedAnsweringLimits {
        requests: finite(2),
        tool_calls: finite(3),
        elapsed_seconds: finite(7),
        known_tokens: finite(11),
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

/// Test 3a — the chat's policy is all four of `[limits.session]`'s dimensions.
#[test]
fn a_chat_turn_is_bounded_by_the_session_question_budget() {
    let plugin = chat_plugin_over(session());

    assert_eq!(
        plugin.turn_policy(&context(RunSurface::Chat)),
        Some(TurnPolicy {
            max_requests: Some(2),
            max_tool_calls: Some(3),
            max_known_tokens: Some(11),
            max_elapsed: Some(Duration::from_secs(7)),
        }),
        "the chat's whole question budget, from [limits.session]"
    );
}

/// Test 3b — no job surface is asked for a policy, whether the plugin was
/// built for the chat or for a job: a job keeps its progress guards and its
/// `[limits.job]` backstops.
#[test]
fn a_job_surface_asks_for_no_turn_policy() {
    let chat = chat_plugin_over(session());
    for surface in [
        RunSurface::Headless,
        RunSurface::Background,
        RunSurface::Helper,
    ] {
        assert_eq!(
            chat.turn_policy(&context(surface)),
            None,
            "a {surface:?} run is not a question: it keeps its own bounds"
        );
    }
}

/// Test 3c — an unlimited session asks for no ceiling on that dimension: an
/// absent dimension is absence, not a zero-request turn.
#[test]
fn an_unlimited_session_dimension_is_asked_for_as_absent() {
    let mut runtime = session_runtime();
    runtime.resolved.limits.session.requests = saya_config::BudgetLimit::Unlimited;
    let plugin = chat_plugin_over(SessionBudget::of(&runtime));

    let policy = plugin
        .turn_policy(&context(RunSurface::Chat))
        .expect("the chat still asks for its other dimensions");
    assert_eq!(policy.max_requests, None, "unlimited is no ceiling");
    assert_eq!(policy.max_tool_calls, Some(3), "the others are unchanged");
}

/// The product defaults a chat runs under with nothing configured are the
/// session's own: 24 requests, 64 tool calls, 250 000 known tokens, 300 s.
#[test]
fn an_unconfigured_chat_asks_for_the_default_session_budget() {
    let plugin = chat_plugin_over(chat_session());

    assert_eq!(
        plugin.turn_policy(&context(RunSurface::Chat)),
        Some(TurnPolicy {
            max_requests: Some(24),
            max_tool_calls: Some(64),
            max_known_tokens: Some(250_000),
            max_elapsed: Some(Duration::from_secs(300)),
        }),
        "the defaults an unconfigured chat has always answered under"
    );
}
