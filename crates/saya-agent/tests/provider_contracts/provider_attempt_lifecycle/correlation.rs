use super::http::local_server_with_bodies;
use super::support::Records;
use async_trait::async_trait;
use saya_agent::{
    ActorCorrelationDigest, AgentLimits, AgentRunControl, AllowReadOnlyApproval, AnsweringLimits,
    AnsweringScope, CancellationToken, LocalStateEffect, OpenAiCompatibleProvider,
    ProviderSettings, ToolDefinition, ToolEffect, ToolError, ToolExecutor, run_agent_with_control,
};
use std::{sync::Arc, time::Duration};

const TOOL_CALL: &str = concat!(
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-secret\",\"type\":\"function\",\"function\":{\"name\":\"secret_tool\",\"arguments\":\"{\\\"api_key\\\":\\\"TOOL_ARGS_SECRET_SENTINEL\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2}}\n\n",
    "data: [DONE]\n\n"
);

struct SecretTool;

#[async_trait]
impl ToolExecutor for SecretTool {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({
            "result": "TOOL_RESULT_SECRET_SENTINEL",
            "row": "DB_ROW_SECRET_SENTINEL"
        }))
    }
}

fn secret_tool() -> ToolDefinition {
    ToolDefinition {
        name: "secret_tool".into(),
        description: "returns a secret-shaped fixture".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}

fn control<'a>(
    records: &'a Records,
    approval: &'a AllowReadOnlyApproval,
    scope: AnsweringScope,
    actor: Option<ActorCorrelationDigest>,
) -> AgentRunControl<'a> {
    let control = AgentRunControl::new(
        AgentLimits::default(),
        AnsweringLimits::default(),
        approval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .with_answering_scope(scope)
    .with_physical_effect_admission(records);
    if let Some(actor) = actor {
        control.with_actor_correlation(actor)
    } else {
        control
    }
}

fn request_with_content_sentinels() -> saya_agent::AgentRequest {
    let mut request = Records::request();
    request.prompt = "PROMPT_SECRET_SENTINEL".into();
    request.system_prompt = Some("CONTEXT_SECRET_SENTINEL".into());
    request
}

#[tokio::test]
async fn retries_and_invocations_keep_local_correlation_without_changing_wire_or_storing_content() {
    let records = Arc::new(Records::default());
    let (base_url, server) = local_server_with_bodies(vec![
        (503, ""),
        (200, TOOL_CALL),
        (200, super::http::ANSWER),
        (200, super::http::ANSWER),
        (200, super::http::ANSWER),
        (200, super::http::ANSWER),
    ])
    .await;
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(base_url)).with_retry_delays(vec![Duration::ZERO]),
        Some("AUTH_SECRET_SENTINEL"),
    )
    .unwrap();
    let approval = AllowReadOnlyApproval;
    let shared_scope = AnsweringScope::new(AnsweringLimits::default(), CancellationToken::new());
    let separate_scope = AnsweringScope::new(AnsweringLimits::default(), CancellationToken::new());
    let first_actor = ActorCorrelationDigest::from_bytes([0xaa; 32]);
    let second_actor = ActorCorrelationDigest::from_bytes([0xbb; 32]);

    for (scope, actor) in [
        (shared_scope.clone(), Some(first_actor)),
        (shared_scope, Some(first_actor)),
        (separate_scope, Some(second_actor)),
        (
            AnsweringScope::new(AnsweringLimits::default(), CancellationToken::new()),
            None,
        ),
    ] {
        let output = run_agent_with_control(
            &provider,
            &SecretTool,
            request_with_content_sentinels(),
            vec![secret_tool()],
            control(&records, &approval, scope, actor),
        )
        .await
        .expect("the local provider response completes");
        assert_eq!(output.answer, "answer");
    }

    let bodies = server.await.unwrap();
    assert_eq!(bodies.len(), 6, "the first logical request retries once");
    assert_eq!(bodies[0], bodies[1], "a transport retry reuses its request");
    for body in [&bodies[3], &bodies[4], &bodies[5]] {
        assert_eq!(body, &bodies[0], "new invocations keep the same wire shape");
    }
    let wire = String::from_utf8(bodies[0].clone()).expect("provider body is UTF-8");
    for sentinel in ["PROMPT_SECRET_SENTINEL", "CONTEXT_SECRET_SENTINEL"] {
        assert!(
            wire.contains(sentinel),
            "the request fixture carries {sentinel}"
        );
    }
    assert!(!wire.contains("aaaaaaaa"));
    assert!(!wire.contains("bbbbbbbb"));
    assert!(!wire.contains("correlation"));
    let follow_up_wire = String::from_utf8(bodies[2].clone()).expect("request is UTF-8");
    for sentinel in [
        "TOOL_ARGS_SECRET_SENTINEL",
        "TOOL_RESULT_SECRET_SENTINEL",
        "DB_ROW_SECRET_SENTINEL",
    ] {
        assert!(follow_up_wire.contains(sentinel));
    }
    let wire_value: serde_json::Value = serde_json::from_slice(&bodies[0]).unwrap();
    let wire_cache_key = wire_value["prompt_cache_key"].as_str().unwrap();
    let wire_cache_digest = u64::from_str_radix(wire_cache_key, 16).unwrap();

    let physical = records.physical.lock().unwrap().clone();
    let models = records.models.lock().unwrap().clone();
    assert_eq!(physical.len(), 6);
    assert_ne!(
        physical[0].0, physical[1].0,
        "each physical send has its own permit"
    );
    assert_eq!(physical[0].1.parent(), models[0]);
    assert_eq!(physical[1].1.parent(), models[0]);
    assert_eq!(physical[2].1.parent(), models[1]);
    assert_eq!(physical[3].1.parent(), models[2]);
    assert_eq!(physical[4].1.parent(), models[3]);
    assert_eq!(physical[5].1.parent(), models[4]);
    assert_eq!(physical[0].1.ordinal(), 0);
    assert_eq!(physical[1].1.ordinal(), 1);
    assert_eq!(physical[2].1.ordinal(), 0);

    let observations = records.observations.lock().unwrap();
    let request_metadata = records.request_metadata.lock().unwrap();
    assert_eq!(observations.len(), 6);
    assert_eq!(request_metadata.len(), 6);
    assert!(request_metadata.iter().all(Option::is_some));
    let correlations = observations
        .iter()
        .map(|(_, _, observation)| {
            observation
                .correlation()
                .expect("physical observations carry request correlation")
        })
        .collect::<Vec<_>>();
    for (index, metadata) in request_metadata.iter().enumerate() {
        assert_eq!(
            metadata.as_ref().unwrap().correlation(),
            observations[index].2.correlation(),
            "the admission owner and settled observation share one correlation"
        );
    }
    assert_eq!(
        correlations[0].conversation_id(),
        correlations[1].conversation_id(),
        "transport retries share their conversation"
    );
    assert_eq!(
        correlations[0].invocation_id(),
        correlations[1].invocation_id(),
        "transport retries share their agent invocation"
    );
    assert_eq!(correlations[0].request_ordinal(), 1);
    assert_eq!(correlations[1].request_ordinal(), 1);
    assert_eq!(
        correlations[0].projection_revision(),
        correlations[1].projection_revision()
    );
    assert_eq!(
        correlations[0].projection_size_bytes(),
        correlations[1].projection_size_bytes()
    );
    assert_eq!(
        correlations[2].invocation_id(),
        correlations[0].invocation_id()
    );
    assert_eq!(correlations[2].request_ordinal(), 2);
    assert_ne!(
        correlations[0].projection_revision(),
        correlations[2].projection_revision()
    );
    assert_ne!(
        correlations[0].projection_size_bytes(),
        correlations[2].projection_size_bytes()
    );
    assert_ne!(
        correlations[0].invocation_id(),
        correlations[3].invocation_id(),
        "separate invocations in one answering scope stay distinct"
    );
    assert_eq!(
        correlations[0].conversation_id(),
        correlations[3].conversation_id(),
        "a shared answering scope keeps one conversation identity"
    );
    assert_ne!(
        correlations[3].conversation_id(),
        correlations[4].conversation_id(),
        "different answering scopes cannot be grouped together"
    );
    assert_ne!(
        correlations[4].conversation_id(),
        correlations[5].conversation_id(),
        "invocations without actor IDs still receive distinct local scopes"
    );
    assert_eq!(
        correlations[0].actor_digest().unwrap().as_bytes(),
        &[0xaa; 32]
    );
    assert_eq!(
        correlations[4].actor_digest().unwrap().as_bytes(),
        &[0xbb; 32]
    );
    assert_eq!(correlations[5].actor_digest(), None);
    for (index, correlation) in correlations.iter().enumerate() {
        assert!(correlation.projection_revision() > 0);
        assert_eq!(correlation.projection_size_bytes(), bodies[index].len());
        assert_eq!(correlation.cache_key_digest(), Some(wire_cache_digest));
    }
    for (index, (_, _, observation)) in observations.iter().enumerate() {
        let rendered = format!("{observation:?}");
        for sentinel in [
            "PROMPT_SECRET_SENTINEL",
            "CONTEXT_SECRET_SENTINEL",
            "TOOL_ARGS_SECRET_SENTINEL",
            "TOOL_RESULT_SECRET_SENTINEL",
            "DB_ROW_SECRET_SENTINEL",
            "AUTH_SECRET_SENTINEL",
        ] {
            assert!(
                !rendered.contains(sentinel),
                "observation leaked {sentinel}"
            );
        }
        if index == 2 {
            assert!(!rendered.contains("call-secret"));
        }
    }
}
