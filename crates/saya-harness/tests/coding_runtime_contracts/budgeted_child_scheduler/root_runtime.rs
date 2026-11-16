//! Actual parent loop delegates three concurrent native children under one ledger.
use super::{
    child_scheduler::{caps, node},
    child_support::tool_definition,
    fixture::{Fixture, limits},
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};
use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentLimits, AgentRequest, AgentRunControl, AllowReadOnlyApproval, AnsweringLimits,
    CancellationToken, NoopEventSink, OpenAiCompatibleProvider, ProviderSettings, ToolError,
    ToolExecutor, run_agent_with_control,
};
use saya_harness::coding::{
    budget::JobBudgetLedger,
    coordinator::{ScheduleReport, ScheduledChild, run_schedule},
};
use saya_types::{CodingEventKind, Sha256Digest};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};

struct Delegate {
    ledger: JobBudgetLedger,
    nodes: Mutex<Option<Vec<ScheduledChild>>>,
    report: Mutex<Option<ScheduleReport>>,
}
#[async_trait]
impl ToolExecutor for Delegate {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let nodes = self
            .nodes
            .lock()
            .await
            .take()
            .expect("one parent delegate call");
        let (progress, mut receiver) = mpsc::channel(128);
        let (report, ()) = tokio::join!(
            run_schedule(
                nodes,
                caps(),
                &self.ledger,
                CancellationToken::new(),
                progress
            ),
            async { while receiver.recv().await.is_some() {} },
        );
        let report = report.unwrap();
        let completed = report
            .outcomes
            .iter()
            .filter(|outcome| outcome.result.is_ok())
            .count();
        *self.report.lock().await = Some(report);
        Ok(serde_json::json!({"completed_children": completed,"debug":"DB_ROW_SECRET_SENTINEL"}))
    }
}

#[tokio::test]
async fn parent_and_three_useful_children_share_exact_tight_and_ample_limits() {
    for cap in [4, 5] {
        let fixture = Fixture::new(&format!("root-three-{cap}")).await;
        let ledger = fixture
            .ledger(limits(cap, 8, 8, 4, 3, Duration::from_secs(20)))
            .await
            .unwrap();
        ledger
            .bind_runtime(1, &Sha256Digest::parse(&"a".repeat(64)).unwrap())
            .await
            .unwrap();
        let mut parent = LocalOpenAi::new().await;
        let mut a = LocalOpenAi::new().await;
        let mut b = LocalOpenAi::new().await;
        let mut c = LocalOpenAi::new().await;
        let children = vec![
            node(&fixture, &a, "agent-a", vec![], "a", Default::default()).await,
            node(&fixture, &b, "agent-b", vec![], "b", Default::default()).await,
            node(&fixture, &c, "agent-c", vec![], "c", Default::default()).await,
        ];
        let tools = Arc::new(Delegate {
            ledger: ledger.clone(),
            nodes: Mutex::new(Some(children)),
            report: Mutex::new(None),
        });
        let task_tools = tools.clone();
        let provider = OpenAiCompatibleProvider::new(
            ProviderSettings::new("fixture", Some(parent.endpoint.clone()))
                .with_retry_delays(vec![]),
            Some("AUTH_SECRET_SENTINEL"),
        )
        .unwrap();
        let view = ledger.root_view();
        let root_actor = super::fixture::actor_digest(&["root", fixture.job.as_str()]);
        let task = tokio::spawn(async move {
            let control = AgentRunControl::new(
                AgentLimits {
                    max_turns: Some(3),
                    ..AgentLimits::default()
                },
                AnsweringLimits {
                    max_requests: Some(10),
                    max_tool_calls: Some(8),
                    max_reported_tokens: None,
                },
                &AllowReadOnlyApproval,
                &NoopEventSink,
                CancellationToken::new(),
            )
            .with_physical_effect_admission(&view)
            .with_actor_correlation(root_actor);
            run_agent_with_control(
                &provider,
                task_tools.as_ref(),
                AgentRequest {
                    prompt: "coordinate three useful tasks PROMPT_SECRET_SENTINEL".into(),
                    profile_names: vec![],
                    model: "fixture".into(),
                    system_prompt: None,
                    history: vec![],
                    context_blocks: vec![],
                },
                vec![tool_definition()],
                control,
            )
            .await
        });
        assert_eq!(parent.request().await, Some(1));
        parent.respond_sse(TOOL_RESPONSE_WITH_SECRET_ARGUMENT);
        // No child receives its response before every provider has actually started.
        assert_eq!(a.request().await, Some(1));
        assert_eq!(b.request().await, Some(1));
        assert_eq!(c.request().await, Some(1));
        assert_eq!(ledger.snapshot().await.child_slots_active, 3);
        for server in [&a, &b, &c] {
            server.respond_sse(ANSWER_RESPONSE);
        }
        if cap == 5 {
            assert_eq!(parent.request().await, Some(2));
            parent.respond_sse(ANSWER_RESPONSE);
        }
        let output = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        if cap == 5 {
            assert_eq!(output.unwrap().answer, "completed");
        } else {
            assert!(matches!(output, Err(AgentError::EffectAdmissionDenied)));
        }
        let report = tools.report.lock().await.take().unwrap();
        assert!(report.stop.is_none());
        assert_eq!(report.outcomes.len(), 3);
        assert!(report.outcomes.iter().all(|outcome| {
            outcome
                .result
                .as_ref()
                .is_ok_and(|output| output.answer == "completed")
                && outcome.completion_sequence.is_some()
        }));
        let snapshot = ledger.snapshot().await;
        assert_eq!(snapshot.logical_requests, cap);
        assert_eq!(snapshot.transport_attempts, cap);
        assert_eq!(snapshot.child_slots_active, 0);
        assert_eq!(snapshot.model_slots_active, 0);
        assert_eq!(
            parent.count.load(std::sync::atomic::Ordering::SeqCst),
            (cap - 3) as usize
        );
        assert_eq!(
            fixture
                .store
                .list_coding_completions(&fixture.job, None, 32)
                .await
                .unwrap()
                .items
                .len(),
            3
        );
        let spans = fixture
            .store
            .list_coding_transport_attempts(&fixture.job, None, 100)
            .await
            .unwrap();
        assert_eq!(spans.items.len() as u64, cap);
        assert!(spans.items.iter().all(|item| item.timing_complete()));
        let child_spans: Vec<_> = spans
            .items
            .iter()
            .filter_map(|item| item.span.as_ref())
            .filter(|span| span.data().agent_id.is_some())
            .collect();
        assert_eq!(child_spans.len(), 3);
        let expected_root_digest =
            digest_hex(super::fixture::actor_digest(&["root", fixture.job.as_str()]).as_bytes());
        let root_spans: Vec<_> = spans
            .items
            .iter()
            .filter_map(|item| item.span.as_ref())
            .filter(|span| span.data().agent_id.is_none())
            .collect();
        assert_eq!(root_spans.len(), if cap == 5 { 2 } else { 1 });
        let expected_root = Sha256Digest::parse(&expected_root_digest).unwrap();
        assert!(root_spans.iter().all(|span| {
            let data = span.data();
            data.job_id == fixture.job
                && data.conversation_correlation_id.is_some()
                && data.invocation_correlation_id.is_some()
                && data.actor_correlation_sha256.as_ref() == Some(&expected_root)
                && data.request_ordinal.is_some()
                && data.projection_revision.is_some()
                && data.projection_size_bytes.is_some()
                && data.cache_key_digest.is_some()
        }));
        if cap == 5 {
            assert_eq!(
                root_spans[0].data().conversation_correlation_id,
                root_spans[1].data().conversation_correlation_id
            );
            assert_eq!(
                root_spans[0].data().invocation_correlation_id,
                root_spans[1].data().invocation_correlation_id
            );
            assert_eq!(root_spans[0].data().request_ordinal.unwrap().get(), 1);
            assert_eq!(root_spans[1].data().request_ordinal.unwrap().get(), 2);
        }
        let mut child_actors = std::collections::BTreeSet::new();
        let mut child_conversations = std::collections::BTreeSet::new();
        let mut child_invocations = std::collections::BTreeSet::new();
        for span in &child_spans {
            let data = span.data();
            assert_eq!(&data.job_id, &fixture.job);
            let agent = data.agent_id.as_ref().unwrap();
            let invocation = data.invocation_id.as_ref().unwrap();
            let expected = digest_hex(
                super::fixture::actor_digest(&[
                    "child",
                    fixture.job.as_str(),
                    agent.as_str(),
                    invocation.as_str(),
                ])
                .as_bytes(),
            );
            assert_eq!(
                data.actor_correlation_sha256.as_ref().unwrap().as_str(),
                expected
            );
            assert!(data.conversation_correlation_id.is_some());
            assert!(data.invocation_correlation_id.is_some());
            assert_eq!(data.request_ordinal.unwrap().get(), 1);
            assert!(data.projection_revision.unwrap().get() > 0);
            assert!(data.projection_size_bytes.unwrap().get() > 0);
            assert!(data.cache_key_digest.is_some());
            child_actors.insert(expected);
            child_conversations.insert(data.conversation_correlation_id.unwrap().get());
            child_invocations.insert(data.invocation_correlation_id.unwrap().get());
        }
        assert_eq!(child_actors.len(), 3);
        assert_eq!(child_conversations.len(), 3);
        assert_eq!(child_invocations.len(), 3);
        let event_page = fixture
            .store
            .list_coding_events(&fixture.job, None, 100)
            .await
            .unwrap();
        for event in event_page
            .events
            .iter()
            .filter(|event| matches!(event.event(), CodingEventKind::TransportObserved { .. }))
        {
            let bytes = event.to_json_bytes().unwrap();
            let wire = String::from_utf8(bytes).unwrap();
            for sentinel in [
                "PROMPT_SECRET_SENTINEL",
                "CHILD_PROMPT_SECRET_SENTINEL",
                "TOOL_ARGS_SECRET_SENTINEL",
                "DB_ROW_SECRET_SENTINEL",
                "AUTH_SECRET_SENTINEL",
            ] {
                assert!(
                    !wire.contains(sentinel),
                    "transport event leaked {sentinel}"
                );
            }
        }
        let domain = &child_spans[0].data().clock_domain;
        assert!(
            child_spans
                .iter()
                .all(|span| &span.data().clock_domain == domain)
        );
        let latest_start = child_spans
            .iter()
            .map(|span| span.data().dispatch_started_ns.unwrap())
            .max()
            .unwrap();
        let earliest_finish = child_spans
            .iter()
            .map(|span| span.data().finished_ns)
            .min()
            .unwrap();
        assert!(
            latest_start < earliest_finish,
            "actual client send/stream intervals overlap"
        );
        parent.shutdown().await;
        a.shutdown().await;
        b.shutdown().await;
        c.shutdown().await;
        fixture.close().await;
    }
}

const TOOL_RESPONSE_WITH_SECRET_ARGUMENT: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[",
    "{\"index\":0,\"id\":\"call-0\",\"function\":{\"name\":\"inspect\",\"arguments\":\"{\\\"value\\\":\\\"TOOL_ARGS_SECRET_SENTINEL\\\"}\"}}",
    "]},\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: [DONE]\n\n",
);

fn digest_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
