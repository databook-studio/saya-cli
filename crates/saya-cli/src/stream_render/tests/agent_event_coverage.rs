// Every `AgentEvent` variant that exists today must map to a real
// `TerminalEvent` or to nothing — never to the loud `NotImplemented`
// catch-all. `ProviderContinuation` is the one this test was added with; the
// list is every sibling so a future variant is added here beside them.

/// Every variant of [`AgentEvent`]. The enum is `#[non_exhaustive]`, so the
/// compiler cannot force this list; the test is the seam that notices a new
/// variant printing `not_implemented` into a headless stream.
fn every_agent_event() -> Vec<AgentEvent> {
    vec![
        AgentEvent::answering_budget_stopped(saya_agent::BudgetDimension::LogicalRequests),
        AgentEvent::assistant_text("the answer"),
        AgentEvent::reasoning_text("thinking"),
        AgentEvent::turn_reset(),
        AgentEvent::turn_started(),
        AgentEvent::provider_recovery(
            saya_agent::ProviderRecoveryPhase::Retrying,
            saya_agent::ProviderRecoveryReason::StreamEnded,
            1,
            4,
        ),
        AgentEvent::provider_continuation(
            saya_agent::ProviderContinuationPhase::Continuing,
            1,
            3,
        ),
        AgentEvent::provider_continuation(
            saya_agent::ProviderContinuationPhase::Exhausted,
            3,
            3,
        ),
        AgentEvent::stream_cut_recovered(1, 2, 3),
        AgentEvent::tool_requested("read", serde_json::json!({}), None),
        AgentEvent::ToolCompleted {
            name: "read".to_owned(),
            summary: "ok".to_owned(),
        },
        AgentEvent::ToolDenied {
            name: "read".to_owned(),
            reason: "refused".to_owned(),
        },
        AgentEvent::knowledge_supplied(KnowledgeOutcome::Off, Vec::new(), 0),
        AgentEvent::knowledge_proposed(saya_agent::ProposedClaimDto {
            claim_id: ClaimId::parse("c-1").unwrap(),
            profile: "analytics".to_owned(),
            object: "catalog.public.orders".to_owned(),
            kind: "table_alias".to_owned(),
            value: "orders".to_owned(),
            column: None,
            status: ClaimStatus::Candidate,
        }),
        AgentEvent::KnowledgeLearningStarted,
        AgentEvent::knowledge_overridden(Vec::new()),
        AgentEvent::knowledge_learning_skipped(LearningSkipReason::Failed),
        AgentEvent::knowledge_learning_disabled("glm-5.2", 2),
        AgentEvent::clarification_needed("which grain?", Vec::new()),
        AgentEvent::complete(),
        AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            saya_agent::TokenUsage::new(1, 2),
        ),
        AgentEvent::failed_attempt_usage(saya_agent::TokenUsage::new(1, 2)),
        AgentEvent::answer_designated("SELECT 1"),
        AgentEvent::consensus_decided(None, 1, 1, 1, 0, false, false),
    ]
}

#[test]
fn every_agent_event_avoids_the_not_implemented_fallback() {
    for event in every_agent_event() {
        if let Some(terminal) = terminal_event(event.clone()) {
            assert!(
                !matches!(terminal, TerminalEvent::NotImplemented { .. }),
                "{event:?} fell through to NotImplemented: {terminal:?}"
            );
        }
    }
}

/// A bounded provider continuation is progress, not content: the headless
/// renderer says nothing for it rather than printing
/// `Not implemented: unrecognized agent event` under the answer.
#[test]
fn provider_continuation_renders_nothing() {
    for phase in [
        saya_agent::ProviderContinuationPhase::Continuing,
        saya_agent::ProviderContinuationPhase::Exhausted,
    ] {
        assert!(
            terminal_event(AgentEvent::provider_continuation(phase, 1, 3)).is_none(),
            "a continuation transition must not reach the headless renderer"
        );
    }
}
