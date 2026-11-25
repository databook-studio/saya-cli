//! `/usage`'s denominator is the window of the model the session is on, never
//! the launch-time model the runtime was composed with. A configured override
//! still wins over the table.

use std::collections::BTreeMap;
use std::path::PathBuf;

use saya_config::{ConnectionsFile, ResolutionInput, resolve};

use super::answer;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::state::SessionState;
use crate::slash::UsageRequest;

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
        cache_scope: PathBuf::from("/tmp/saya-ops-usage-tests"),
        investigations_root: PathBuf::from("/tmp/saya-ops-usage-tests/investigations"),
        secret_values: BTreeMap::new(),
    }
}

/// The share is measured against the session's model, not the one the launch
/// resolved: `/model` moved the former and left the latter naming the old one.
#[test]
fn the_context_share_uses_the_session_model_not_the_launch_model() {
    let runtime = runtime_for("claude-sonnet-5", None);
    let mut state = SessionState::new("s", None, "gpt-4o");
    state.usage.note_request(Some(64_000), None);

    let panel = answer(&mut state, &runtime, UsageRequest::show());

    assert!(
        panel.contains("Context: 50% of 128000 tokens"),
        "the session model's window is the denominator:\n{panel}"
    );
}

/// A configured window is a fact about the deployment, so it still overrides
/// the table row for whichever model the session is on.
#[test]
fn a_configured_window_still_overrides_the_table_row() {
    let runtime = runtime_for("claude-sonnet-5", Some(64_000));
    let mut state = SessionState::new("s", None, "gpt-4o");
    state.usage.note_request(Some(32_000), None);

    let panel = answer(&mut state, &runtime, UsageRequest::show());

    assert!(
        panel.contains("Context: 50% of 64000 tokens"),
        "the stated window is the denominator:\n{panel}"
    );
}
