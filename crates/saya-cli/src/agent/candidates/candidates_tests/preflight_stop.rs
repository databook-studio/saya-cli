//! D3 on `saya ask` — the preflight refusal reaches the question's own token.
//!
//! The old body's row
//! (`agent/runtime_preflight_tests.rs::the_shared_question_scope_survives_a_preflight_refusal`)
//! stated two things about one refused question: "the token's first reason is
//! the preflight budget", and the scope is still usable afterwards
//! (`begin_segment`/`finish_segment` neither panic nor refuse).
//!
//! The live ask entry is [`run_with_candidates`], and it needs nothing new for
//! this: its `LiveAttemptRunner` `Stopped` arm already transcribes a turn's
//! budget stop onto the question's token. What it could not do was see the
//! stop — a preflight-refused turn reported `Answered`, so the arm never ran.
//! This is the same question, asked through the live entry: one attempt, no
//! provider request, and the question's token naming the preflight dimension
//! first while its scope keeps working.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use saya_agent::{
    AgentEvent, AgentEventSink, ApprovalPolicy, BudgetDimension, CancellationReason,
    CancellationToken,
};
use saya_config::{ConnectionsFile, ResolutionInput, resolve};

use super::super::run_with_candidates;
use crate::agent::question_scope::QuestionScope;
use crate::agent::turn_config::PromptOverrides;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;

/// Load ceiling, not a product bound: how long the question may take.
const RUN: Duration = Duration::from_secs(120);

/// The window a chat request cannot fit in, and the output reservation it must
/// still leave room for — the two facts a preflight refusal needs.
const REFUSING_WINDOW: u64 = 2_048;
const OUTPUT_RESERVE: u32 = 512;

/// A sink that keeps every event, so "no request was made" is assertable from
/// the surface's side too.
#[derive(Default)]
struct Recording(Arc<std::sync::Mutex<Vec<AgentEvent>>>);

#[async_trait::async_trait]
impl AgentEventSink for Recording {
    async fn emit(&self, event: AgentEvent) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

impl Recording {
    fn budget_stops(&self) -> Vec<BudgetDimension> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|event| match event {
                AgentEvent::AnsweringBudgetStopped { dimension } => Some(*dimension),
                _ => None,
            })
            .collect()
    }
}

/// The folder this row asks in: a fresh one under the workspace's own temp
/// tree, so the run's lease and records never touch it.
fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-candidates-preflight-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the fixture root creates");
    root
}

/// The ask's runtime, stated at the scripted provider's endpoint with the
/// refusing window: the two facts the preflight reads.
fn refusing_runtime(root: &Path, endpoint: &str) -> RuntimeConfig {
    let connections = ConnectionsFile::default();
    let mut runtime = RuntimeConfig {
        resolved: resolve(ResolutionInput::new(connections.clone())).expect("test config resolves"),
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: root.to_path_buf(),
        investigations_root: root.join("investigations"),
        secret_values: BTreeMap::new(),
    };
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.model = "composition".into();
    runtime.resolved.ai.base_url = Some(endpoint.into());
    runtime.resolved.ai.api_key = None;
    runtime.resolved.ai.allow_data_sharing = true;
    runtime.resolved.ai.retry_delays_ms.clear();
    runtime.resolved.ai.timeout_seconds = 5;
    runtime.resolved.ai.idle_timeout_seconds = 5;
    runtime.resolved.ai.max_output_tokens = Some(OUTPUT_RESERVE);
    runtime.resolved.ai.context_window_tokens = Some(REFUSING_WINDOW);
    runtime.resolved.memory.mode = saya_config::MemoryMode::Off;
    let effective = runtime
        .resolved
        .endpoints
        .get_mut(saya_config::ORCHESTRATOR_ROLE)
        .expect("the runtime resolves an orchestrator endpoint");
    effective.provider = saya_config::AiProvider::OpenaiCompatible;
    effective.model = "composition".into();
    effective.base_url = Some(endpoint.into());
    effective.api_key = None;
    runtime
}

#[tokio::test]
async fn a_refused_question_records_the_preflight_reason_and_keeps_its_scope() {
    let root = scratch("stop");
    let provider =
        Scripted::start_with_script(vec![Reply::Text("the answer that was never asked for")]);
    let runtime = refusing_runtime(&root, &provider.endpoint);
    let token = CancellationToken::new();
    let scope = QuestionScope::ask(&runtime, token.clone());
    let sink = Arc::new(Recording::default());

    let asked = tokio::time::timeout(
        RUN,
        run_with_candidates(
            &runtime,
            "what is the total revenue?",
            ApprovalPolicy::ReadOnly,
            false,
            PromptOverrides::default(),
            root.clone(),
            RenderFormat::Text,
            Arc::clone(&sink) as Arc<dyn AgentEventSink>,
            scope.clone(),
            Some(saya_store::SqliteStateStore::new(
                root.join("state.sqlite3"),
            )),
            1,
        ),
    )
    .await
    .expect("a refused question settles");
    let output = asked.expect("a preflight refusal is a partial answer, not an error");
    assert!(
        output.truncated,
        "the refusal is partial output: {output:?}"
    );
    assert_eq!(
        provider.arrived(),
        0,
        "a too-small known window refuses before any provider request"
    );
    assert_eq!(
        sink.budget_stops(),
        vec![BudgetDimension::RequestContextWindow],
        "exactly one RequestContextWindow stop: {:?}",
        sink.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    );
    assert_eq!(
        token.reason(),
        Some(CancellationReason::Budget(
            BudgetDimension::RequestContextWindow
        )),
        "the token's first reason is the preflight budget"
    );
    // The scope stays usable: the refusal is the turn's, and the question that
    // asked it may open the next segment of its own clock. `begin_segment`
    // refuses an overlapping segment and latches an elapsed stop only when the
    // clock is spent, so these calls not panicking and the reason surviving
    // them is the row's "the scope survives".
    let _remaining = scope.begin_segment();
    scope.finish_segment();
    assert_eq!(
        token.reason(),
        Some(CancellationReason::Budget(
            BudgetDimension::RequestContextWindow
        )),
        "a later segment never overwrites the first reason"
    );
    let _ = std::fs::remove_dir_all(root);
}
