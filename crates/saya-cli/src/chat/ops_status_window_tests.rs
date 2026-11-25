//! The panel's two reported figures: how much of the model's window the last
//! request used, and what the session has cost so far.
//!
//! Both are reported, never inferred: an unknown window and an unreported cost
//! each keep their line's "unknown" wording, and a reported zero is a figure.

use std::collections::BTreeMap;
use std::path::PathBuf;

use saya_agent::TokenUsage;
use saya_config::{ConnectionsFile, ResolutionInput, resolve};
use saya_store::FsSessionStore;

use super::Section;
use super::{StatusFacts, StatusPanel};
use crate::chat::ops_status_facts::{context_use, cost_fact};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::SessionUsage;
use crate::interactive::session::state::SessionState;

/// A runtime whose launch-time model is `model`, with the stated override.
fn runtime_for(model: &str, window: Option<u64>) -> RuntimeConfig {
    let connections = ConnectionsFile::default();
    let mut resolved = resolve(ResolutionInput::new(connections.clone())).expect("resolves");
    resolved.ai.model = model.to_owned();
    resolved.ai.context_window_tokens = window;
    RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-ops-status-window-tests"),
        investigations_root: PathBuf::from("/tmp/saya-ops-status-window-tests/investigations"),
        secret_values: BTreeMap::new(),
    }
}

fn usage(input: u64, cost: Option<u64>) -> SessionUsage {
    let mut usage = SessionUsage::default();
    let mut call = TokenUsage::new(input, 10);
    call.cost_micros = cost;
    usage.answering.record_call(&call);
    usage.note_request(Some(input), cost);
    usage
}

/// A known window words the share of it the last request used.
#[test]
fn a_known_window_words_the_share() {
    let usage = usage(51_200, Some(31_200));

    assert_eq!(
        context_use(&usage, Some(128_000)).as_deref(),
        Some("40% of a 128000-token window")
    );
}

/// An unknown window stays unknown, and so does a session that has made no
/// request: neither is dressed up as a share.
#[test]
fn an_unknown_window_or_no_request_stays_unknown() {
    assert_eq!(context_use(&usage(51_200, None), None), None);
    assert_eq!(context_use(&SessionUsage::default(), Some(128_000)), None);
}

/// The cost line is the reported spend, and absent until one is reported — a
/// reported zero shows as zero.
#[test]
fn the_cost_line_is_the_reported_spend() {
    assert_eq!(
        cost_fact(&usage(10, Some(31_200))).as_deref(),
        Some("$0.0312")
    );
    assert_eq!(
        cost_fact(&usage(10, None)),
        None,
        "an unreported cost is not $0"
    );
    assert_eq!(cost_fact(&usage(10, Some(0))).as_deref(), Some("$0"));
}

/// The panel says both, under the Model heading, exactly where the context line
/// used to say "unknown".
#[test]
fn the_model_section_states_both_figures() {
    let facts = StatusFacts {
        provider: "anthropic".into(),
        model: "sonnet".into(),
        context_use: context_use(&usage(64_000, Some(12_500)), Some(128_000)),
        cost: cost_fact(&usage(64_000, Some(12_500))),
        ..StatusFacts::default()
    };
    let model = StatusPanel::build(&facts).render_section(Section::Model);

    assert!(
        model.contains("\n  Context use: 50% of a 128000-token window\n"),
        "{model}"
    );
    assert!(model.contains("\n  Cost: $0.0125"), "{model}");
}

/// `/status` divides by the window of the model the session is on, not the
/// launch-time model the runtime still names.
#[test]
fn the_status_context_line_uses_the_session_model() {
    let runtime = runtime_for("claude-sonnet-5", None);
    let mut state = SessionState::new("s", None, "gpt-4o");
    state.usage.note_request(Some(64_000), None);
    let store = FsSessionStore::new(PathBuf::from("/tmp/saya-ops-status-window-tests/store"));

    let facts = StatusFacts::of_session(&state).with_runtime(&state, &runtime, &store, None);

    assert_eq!(
        facts.context_use.as_deref(),
        Some("50% of a 128000-token window"),
        "the session model's window is the denominator"
    );
}

/// A session that reported nothing names why, rather than showing a share of a
/// window nobody measured and a spend nobody stated.
#[test]
fn an_unreported_session_names_why() {
    let model = StatusPanel::build(&StatusFacts::default()).render_section(Section::Model);

    assert!(
        model.contains(
            "\n  Context use: unknown — no answering call has reported a window this session\n"
        ),
        "{model}"
    );
    assert!(
        model.contains("\n  Cost: unknown — no answering call has reported a cost this session"),
        "{model}"
    );
}
