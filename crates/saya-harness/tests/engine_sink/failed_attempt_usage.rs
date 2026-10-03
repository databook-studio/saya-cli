use std::time::Instant;
use std::{collections::VecDeque, sync::atomic::AtomicUsize};

use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentLimits, AgentRequest, CancellationToken, ChatProvider, ChatRequest,
    ChatResponse, ProviderError, ProviderEvent, ProviderStream, TokenUsage, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use saya_harness::{
    engine::{
        EngineEventSink, EpisodeCollaborators, EpisodeRequest, IncompletePolicy, ManifestBounds,
        ResumeError, ResumeRun, RunState, SinkBudgets, TransitionEvent, UsageTotals, resume,
    },
    journal::Journal,
    workspace::Workspace,
};
use saya_types::{Capabilities, RunEvent, RunPlan, StepSpec};
use std::sync::{Arc, Mutex};

struct ScriptedProvider {
    streams: Mutex<VecDeque<Vec<ProviderEvent>>>,
    requests: AtomicUsize,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "receipt-source"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the test drives provider streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let events = self.streams.lock().unwrap().pop_front().unwrap();
        Ok(Box::pin(futures_util::stream::iter(
            events.into_iter().map(Ok),
        )))
    }
}

#[derive(Default)]
struct NoTools(AtomicUsize);

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(serde_json::Value::Null)
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "bounded receipt test".into(),
        profile_names: Vec::new(),
        model: "scripted".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn success(usage: TokenUsage) -> Vec<ProviderEvent> {
    vec![
        ProviderEvent::Usage(usage),
        ProviderEvent::TextDelta("done".into()),
        ProviderEvent::Done,
    ]
}

fn terminal_collection_limit(usage: Option<TokenUsage>) -> Vec<ProviderEvent> {
    let mut events = usage
        .into_iter()
        .map(ProviderEvent::Usage)
        .collect::<Vec<_>>();
    events.push(ProviderEvent::ToolCalls(
        (0..257)
            .map(|id| saya_agent::ToolCall {
                id: format!("call-{id}"),
                name: "unexecuted".into(),
                arguments: serde_json::Value::Null,
            })
            .collect(),
    ));
    events.push(ProviderEvent::Done);
    events
}

#[tokio::test]
async fn actual_failed_attempt_receipt_is_journaled_and_carried_by_resume() {
    let root = super::temp_root("failed-attempt-usage");
    let (store, run_id) =
        super::seeded_store(&root, "failed-attempt-usage", super::RunStatus::Approved).await;
    let run_dir = root.join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    journal
        .append(&RunEvent::PlanApproved { scopes: None })
        .unwrap();
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Approved,
        journal.clone(),
        store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: Some(40),
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );
    sink.record(TransitionEvent::Begin).await.unwrap();
    journal.append(&RunEvent::StepStarted { step: 0 }).unwrap();

    let provider = ScriptedProvider {
        streams: Mutex::new(
            vec![
                success(TokenUsage::new(30, 0).with_cached_input(Some(5))),
                terminal_collection_limit(Some(
                    TokenUsage::new(14, 0).with_cache_creation(Some(0)),
                )),
                terminal_collection_limit(None),
                vec![
                    ProviderEvent::TextDelta("resumed".into()),
                    ProviderEvent::Done,
                ],
            ]
            .into(),
        ),
        requests: AtomicUsize::new(0),
    };
    let tools = NoTools::default();
    let cancellation = CancellationToken::default();

    let output = run_agent_with_sink(
        &provider,
        &tools,
        request(),
        Vec::new(),
        AgentLimits::default(),
        &saya_agent::AllowReadOnlyApproval,
        &sink,
        cancellation.clone(),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "done");
    let terminal = run_agent_with_sink(
        &provider,
        &tools,
        request(),
        Vec::new(),
        AgentLimits::default(),
        &saya_agent::AllowReadOnlyApproval,
        &sink,
        cancellation,
    )
    .await;
    assert!(matches!(
        terminal,
        Err(AgentError::Provider(ProviderError::ToolCollectionLimit))
    ));
    let unreported = run_agent_with_sink(
        &provider,
        &tools,
        request(),
        Vec::new(),
        AgentLimits::default(),
        &saya_agent::AllowReadOnlyApproval,
        &sink,
        CancellationToken::default(),
    )
    .await;
    assert!(matches!(
        unreported,
        Err(AgentError::Provider(ProviderError::ToolCollectionLimit))
    ));

    let events = journal.read().unwrap();
    let usage = events
        .iter()
        .filter(|event| matches!(event, RunEvent::Usage { .. }))
        .collect::<Vec<_>>();
    assert_eq!(usage.len(), 2, "one per actual settled receipt: {events:?}");
    assert!(matches!(
        usage[0],
        RunEvent::Usage {
            tokens: Some(30),
            cached_input_tokens: Some(5),
            cache_creation_input_tokens: None,
            ..
        }
    ));
    assert!(matches!(
        usage[1],
        RunEvent::Usage {
            tokens: Some(14),
            cached_input_tokens: None,
            cache_creation_input_tokens: Some(0),
            ..
        }
    ));
    assert_eq!(
        UsageTotals::from_journal(&events).carried_tokens,
        44,
        "the successful usage and failed attempt each count once"
    );
    assert_eq!(tools.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(
        provider.requests.load(std::sync::atomic::Ordering::Relaxed),
        3
    );

    let step = StepSpec::new(
        "retry bounded work",
        Capabilities::default(),
        None,
        Vec::new(),
        None,
    )
    .unwrap();
    let plan = RunPlan::new(vec![step]).unwrap();
    let toolsets = vec![saya_harness::engine::StepToolset {
        executor: Arc::new(NoTools::default()),
        definitions: Vec::new(),
    }];
    let workspace_dir = root.join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    let resumed = resume(
        &run_dir,
        ResumeRun {
            run_id: run_id.clone(),
            store,
            plan,
            incomplete_policy: IncompletePolicy::Retry,
            workspace: Workspace::open(&workspace_dir).unwrap(),
            collaborators: EpisodeCollaborators {
                provider: &provider,
                approval: &saya_agent::AllowReadOnlyApproval,
                toolsets: &toolsets,
                cancellation: CancellationToken::default(),
            },
            request: EpisodeRequest {
                model: "scripted".into(),
                profile_names: Vec::new(),
                memory_allows_candidate_writes: false,
            },
            bounds: ManifestBounds {
                max_files: 8,
                max_file_bytes: 64 * 1024,
            },
            wall_clock: None,
            token_ceiling: Some(40),
            download_budget: None,
            journal_wire: None,
            agent_stream: None,
        },
    )
    .await;
    assert!(matches!(resumed, Err(ResumeError::Episode { .. })));
    let events = journal.read().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                RunEvent::Paused {
                    reason: saya_types::PauseReason::BudgetExhausted
                }
            ))
            .count(),
        2,
        "resume carries the known 44 past the 40-token ceiling: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Usage { .. }))
            .count(),
        2,
        "resume checks the carried total without fabricating another receipt"
    );
    assert_eq!(sink.state(), RunState::Paused);
    let _ = std::fs::remove_dir_all(root);
}
