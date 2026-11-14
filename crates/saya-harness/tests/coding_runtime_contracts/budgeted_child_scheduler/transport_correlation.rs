use super::{
    fixture::{Fixture, actor_digest, limits},
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};
use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AgentRunControl, AllowReadOnlyApproval, AnsweringLimits,
    CancellationToken, NoopEventSink, OpenAiCompatibleProvider, ProviderSettings, ToolError,
    ToolExecutor, run_agent_with_control,
};
use saya_types::{CodingEventKind, Sha256Digest};
use std::time::Duration;

#[tokio::test]
async fn retry_spans_retain_one_request_correlation_and_distinct_physical_attempts() {
    let fixture = Fixture::new("transport-correlation-retry").await;
    let ledger = fixture
        .ledger(limits(1, 2, 1, 1, 1, Duration::from_secs(10)))
        .await
        .unwrap();
    ledger
        .bind_runtime(1, &Sha256Digest::parse(&"a".repeat(64)).unwrap())
        .await
        .unwrap();
    let mut server = LocalOpenAi::new().await;
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("fixture", Some(server.endpoint.clone()))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("AUTH_SECRET_SENTINEL"),
    )
    .unwrap();
    let actor = actor_digest(&["root", fixture.job.as_str()]);
    let view = ledger.root_view();
    let task = tokio::spawn(async move {
        run_agent_with_control(
            &provider,
            &NoTools,
            AgentRequest {
                prompt: "transport retry PROMPT_SECRET_SENTINEL".into(),
                profile_names: Vec::new(),
                model: "fixture".into(),
                system_prompt: None,
                history: Vec::new(),
                context_blocks: Vec::new(),
            },
            Vec::new(),
            AgentRunControl::new(
                AgentLimits {
                    max_turns: Some(1),
                    ..AgentLimits::default()
                },
                AnsweringLimits {
                    max_requests: Some(1),
                    max_tool_calls: Some(1),
                    max_reported_tokens: None,
                },
                &AllowReadOnlyApproval,
                &NoopEventSink,
                CancellationToken::new(),
            )
            .with_physical_effect_admission(&view)
            .with_actor_correlation(actor),
        )
        .await
    });
    assert_eq!(server.request().await, Some(1));
    server.respond_status(500);
    assert_eq!(server.request().await, Some(2));
    server.respond_sse(ANSWER_RESPONSE);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .answer,
        "completed"
    );

    let spans = fixture
        .store
        .list_coding_transport_attempts(&fixture.job, None, 10)
        .await
        .unwrap();
    assert_eq!(spans.items.len(), 2);
    let first_span = spans.items[0].span.as_ref().unwrap();
    let second_span = spans.items[1].span.as_ref().unwrap();
    let first = first_span.data();
    let second = second_span.data();
    assert_eq!(
        first.conversation_correlation_id,
        second.conversation_correlation_id
    );
    assert_eq!(
        first.invocation_correlation_id,
        second.invocation_correlation_id
    );
    assert_eq!(
        first.actor_correlation_sha256,
        second.actor_correlation_sha256
    );
    assert_eq!(first.request_ordinal, second.request_ordinal);
    assert_eq!(first.request_ordinal.unwrap().get(), 1);
    assert_eq!(first.projection_revision, second.projection_revision);
    assert_eq!(first.projection_size_bytes, second.projection_size_bytes);
    assert_eq!(first.cache_key_digest, second.cache_key_digest);
    assert_eq!(first.ordinal, 0);
    assert_eq!(second.ordinal, 1);
    assert_ne!(first.reservation_sequence, second.reservation_sequence);
    assert_eq!(
        first.parent_reservation_sequence,
        second.parent_reservation_sequence
    );
    assert_eq!(
        first.actor_correlation_sha256.as_ref().unwrap().as_str(),
        digest_hex(actor.as_bytes())
    );
    for event in fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap()
        .events
        .iter()
        .filter(|event| matches!(event.event(), CodingEventKind::TransportObserved { .. }))
    {
        let wire = String::from_utf8(event.to_json_bytes().unwrap()).unwrap();
        assert!(!wire.contains("PROMPT_SECRET_SENTINEL"));
        assert!(!wire.contains("AUTH_SECRET_SENTINEL"));
    }
    server.shutdown().await;
    fixture.close().await;
}

fn digest_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::UnsupportedTool)
    }
}
