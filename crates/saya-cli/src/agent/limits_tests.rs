//! The adapter from resolved `[limits]` to the answering engine: the session
//! and `ask` each reach `AnsweringLimits` with their own numbers.

use super::{AnsweringSurface, answering_deadline, answering_limits};
use crate::config::runtime::RuntimeConfig;
use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
use std::time::Duration;

fn runtime(user_toml: &str) -> RuntimeConfig {
    let connections = ConnectionsFile::default();
    let resolved = resolve(
        ResolutionInput::new(connections.clone())
            .with_user(ConfigFile::from_toml(user_toml).expect("fixture parses")),
    )
    .expect("test config resolves");
    RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: std::path::PathBuf::from("/tmp/saya-limits-tests"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: std::collections::BTreeMap::new(),
    }
}

#[test]
fn session_limits_reach_the_engine_verbatim() {
    let runtime = runtime(
        "[limits.session]\nrequests = 8\ntool_calls = 3\n\
         known_tokens = 'unlimited'\nelapsed_seconds = 45\n",
    );
    let limits = answering_limits(&runtime, AnsweringSurface::Session);
    assert_eq!(limits.max_requests, Some(8));
    assert_eq!(limits.max_tool_calls, Some(3));
    assert_eq!(limits.max_reported_tokens, None, "unlimited is no ceiling");
    assert_eq!(
        answering_deadline(&runtime, AnsweringSurface::Session),
        Some(Duration::from_secs(45))
    );
}

#[test]
fn ask_limits_reach_the_engine_and_fall_back_per_key() {
    let runtime = runtime(
        "[limits.session]\nrequests = 8\nelapsed_seconds = 45\n\
         [limits.ask]\nrequests = 2\n",
    );
    let ask = answering_limits(&runtime, AnsweringSurface::Ask);
    assert_eq!(
        ask.max_requests,
        Some(2),
        "ask states its own request ceiling"
    );
    assert_eq!(
        ask.max_tool_calls,
        Some(64),
        "an unstated ask key falls back"
    );
    assert_eq!(
        answering_deadline(&runtime, AnsweringSurface::Ask),
        Some(Duration::from_secs(45)),
        "ask inherits the session's elapsed ceiling"
    );
    assert_eq!(
        answering_limits(&runtime, AnsweringSurface::Session).max_requests,
        Some(8),
        "the session is untouched by the ask table"
    );
}

#[test]
fn an_unlimited_ask_deadline_is_no_deadline() {
    let runtime = runtime("[limits.ask]\nelapsed_seconds = 'unlimited'\n");
    assert_eq!(answering_deadline(&runtime, AnsweringSurface::Ask), None);
    assert_eq!(
        answering_deadline(&runtime, AnsweringSurface::Session),
        Some(Duration::from_secs(300)),
        "the default session ceiling still applies"
    );
}
