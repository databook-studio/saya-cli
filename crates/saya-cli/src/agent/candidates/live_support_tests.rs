//! Shared support for the live candidate-vote proofs: the scripted attempt
//! runner, the recording event sink, the nominating output builder, and the
//! one-second question scope.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AgentOutput, CancellationToken, TokenUsage};
use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};

use crate::agent::candidates::AttemptRunner;
use crate::agent::runtime::AgentRuntimeError;
use crate::agent::question_scope::QuestionScope;
use crate::config::runtime::RuntimeConfig;

/// A scripted [`AttemptRunner`]: returns the queued outputs in order and
/// counts its calls. It never touches the scope, so the question's 1s elapsed
/// ceiling is still whole when the vote segment opens.
pub(super) struct ScriptedRunner {
    outcomes: std::sync::Mutex<Vec<AgentOutput>>,
    calls: std::sync::atomic::AtomicUsize,
}

impl ScriptedRunner {
    pub(super) fn new(outcomes: Vec<AgentOutput>) -> Self {
        Self {
            outcomes: std::sync::Mutex::new(outcomes),
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl AttemptRunner for ScriptedRunner {
    fn run(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<AgentOutput, AgentRuntimeError>> + '_>,
    > {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut outcomes = self.outcomes.lock().expect("outcomes lock");
            assert!(!outcomes.is_empty(), "ScriptedRunner ran out of outputs");
            Ok(outcomes.remove(0))
        })
    }
}

pub(super) struct RecordingSink {
    pub(super) events: std::sync::Mutex<Vec<AgentEvent>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().expect("events lock").push(event);
    }
}

/// An attempt output that nominated `sql`: prose answer, the designation
/// evidence event, and the typed `answer_sql` nomination.
pub(super) fn nominating_output(prose: &str, sql: &str) -> AgentOutput {
    AgentOutput {
        answer: prose.to_string(),
        events: vec![AgentEvent::answer_designated(sql)],
        used_bounded_sql_query: true,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: Some(sql.to_string()),
    }
}

/// A real [`QuestionScope`] whose elapsed ceiling is the minimum: 1s, via a
/// user-layer `[ai]` budget (the trusted layer, so resolution keeps it).
pub(super) fn one_second_scope(cancellation: CancellationToken) -> QuestionScope {
    let connections = ConnectionsFile::default();
    let user = ConfigFile::from_toml("[ai]\ninvestigation_max_elapsed_seconds = 1\n")
        .expect("test user config parses");
    let resolved = resolve(ResolutionInput::new(connections.clone()).with_user(user))
        .expect("test config resolves");
    let runtime = RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: std::path::PathBuf::from("/tmp/saya-live-vote-tests"),
        investigations_root: std::path::PathBuf::from("/tmp/saya-live-vote-tests/investigations"),
        secret_values: std::collections::BTreeMap::new(),
    };
    QuestionScope::new(&runtime, cancellation)
}