//! Cumulative coding context belongs to the loop, never the scripted provider.
mod coding_compaction_support;

use async_trait::async_trait;
use coding_compaction_support::*;
use saya_agent::{
    AgentLimits, AgentRunControl, AllowReadOnlyApproval, AnsweringLimits, ApprovalDecider,
    AuthorityRefusalContext, CancellationToken, ImmutableAuthorityRefusal, ToolDefinition,
    run_agent_with_control, run_agent_with_sink,
};
use std::sync::atomic::{AtomicBool, Ordering};

struct DenyOnlyBlockedPath(AtomicBool);

#[async_trait]
impl ApprovalDecider for DenyOnlyBlockedPath {
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool {
        if tool.name != "write" {
            return true;
        }
        let blocked = arguments["path"] == "src/blocked.rs";
        if blocked {
            self.0.store(true, Ordering::Release);
        }
        !blocked
    }

    fn immutable_authority_refusal(
        &self,
        tool: &ToolDefinition,
        arguments: &serde_json::Value,
    ) -> Option<ImmutableAuthorityRefusal> {
        (tool.name == "write" && arguments["path"] == "src/blocked.rs")
            .then_some(ImmutableAuthorityRefusal::OutsideEffectiveWritePaths)
    }
}

#[tokio::test]
async fn two_compactions_preserve_coding_facts() {
    for concurrent in [false, true] {
        let tools = Files::new();
        let sink = Observations::default();
        let provider = Script::new(concurrent, &sink);
        let output = run_agent_with_sink(
            &provider,
            &tools,
            request(),
            definitions(concurrent),
            AgentLimits {
                context_byte_budget: BUDGET,
                permit_workspace_writes: true,
                ..AgentLimits::default()
            },
            &AllowReadOnlyApproval,
            &sink,
            CancellationToken::new(),
        )
        .await
        .expect("the bounded scripted run completes");
        assert_eq!(output.answer, "done");
        assert!(
            tools.calls.load(Ordering::Acquire) >= 128,
            "padding must actually execute"
        );
        assert!(
            sink.compactions.load(Ordering::Acquire) >= 2,
            "actual compactions: {}",
            sink.compactions.load(Ordering::Acquire)
        );
        let requests = provider.requests.lock().unwrap();
        for (request, _) in requests.iter() {
            let conversation_bytes = request
                .messages
                .iter()
                .map(|message| {
                    message.content.len()
                        + message.role.len()
                        + message
                            .tool_calls
                            .iter()
                            .map(|call| {
                                call.id.len()
                                    + call.name.len()
                                    + serde_json::to_vec(&call.arguments).unwrap().len()
                            })
                            .sum::<usize>()
                })
                .sum::<usize>();
            assert!(conversation_bytes <= BUDGET);
            // ChatRequest also serializes definition effects and the envelope;
            // these are outside the documented conversation-byte estimate.
            let definition_bytes = serde_json::to_vec(&request.tools).unwrap().len();
            assert!(definition_bytes < 4096);
            assert!(serde_json::to_vec(request).unwrap().len() <= BUDGET + definition_bytes + 1024);
        }
        let after_second = requests
            .iter()
            .filter(|(_, compactions)| *compactions >= 2)
            .collect::<Vec<_>>();
        assert!(!after_second.is_empty());
        for (request, _) in after_second {
            let digests = request
                .messages
                .iter()
                .filter(|message| {
                    message.role == "assistant"
                        && message.tool_calls.is_empty()
                        && message.content.starts_with("…[compacted:")
                })
                .collect::<Vec<_>>();
            assert_eq!(digests.len(), 1, "one current checkpoint replaces history");
            let digest = &digests[0].content;
            assert!(digest.contains("src/main.rs"), "early path lost: {digest}");
            for hidden in [
                "FILE_BODY_SENTINEL",
                "EDIT_BODY_SENTINEL",
                "CHILD_BODY_SENTINEL",
                "FORGED_BODY_SENTINEL",
                "forged.rs",
                "PATH_SENTINEL",
            ] {
                assert!(!digest.contains(hidden), "unsafe body entered checkpoint");
            }
        }
        let first_digest = requests
            .iter()
            .flat_map(|(request, _)| &request.messages)
            .find(|message| {
                message.role == "assistant"
                    && message.tool_calls.is_empty()
                    && message.content.starts_with("…[compacted:")
            })
            .unwrap();
        assert!(first_digest.content.contains(BEFORE_SHA));
        let final_digest = requests
            .last()
            .unwrap()
            .0
            .messages
            .iter()
            .find(|message| {
                message.role == "assistant"
                    && message.tool_calls.is_empty()
                    && message.content.starts_with("…[compacted:")
            })
            .unwrap();
        assert!(final_digest.content.contains(AFTER_SHA));
        assert!(!final_digest.content.contains(BEFORE_SHA));
        assert_eq!(
            std::fs::read(tools.root.join("src/main.rs")).unwrap(),
            AFTER
        );
    }
}

#[tokio::test]
async fn authority_refusal_survives_actual_compaction_without_becoming_execution_evidence() {
    let tools = Files::new();
    let sink = Observations::default();
    let provider = Script::new(false, &sink);
    let approval = DenyOnlyBlockedPath(AtomicBool::new(false));
    let context =
        AuthorityRefusalContext::new(saya_types::InvocationId::parse("invocation-a").unwrap(), 7);
    let output = run_agent_with_control(
        &provider,
        &tools,
        request(),
        definitions(false)
            .into_iter()
            .map(|mut definition| {
                if definition.name == "write" {
                    definition.effect.requires_approval = true;
                }
                definition
            })
            .collect(),
        AgentRunControl::new(
            AgentLimits {
                context_byte_budget: BUDGET,
                permit_workspace_writes: true,
                ..AgentLimits::default()
            },
            AnsweringLimits::default(),
            &approval,
            &sink,
            CancellationToken::new(),
        )
        .with_authority_refusal_context(context),
    )
    .await
    .expect("the bounded scripted run completes after the refusal");
    assert_eq!(output.answer, "done");
    assert_eq!(
        std::fs::read(tools.root.join("src/main.rs")).unwrap(),
        AFTER
    );
    assert!(sink.compactions.load(Ordering::Acquire) >= 2);
    let requests = provider.requests.lock().unwrap();
    let digest = requests
        .iter()
        .filter(|(_, compactions)| *compactions >= 2)
        .flat_map(|(request, _)| &request.messages)
        .rfind(|message| message.content.starts_with("…[compacted:"))
        .expect("a real compacted provider request retains the denial");
    assert!(digest.content.contains("authority_refusal"));
    assert!(!digest.content.contains("invocation-a"));
    assert!(digest.content.contains("outside_effective_write_paths"));
    assert!(
        digest
            .content
            .contains("use_exact_effective_write_paths_or_report_to_coordinator")
    );
    assert!(!digest.content.contains("EDIT_BODY_SENTINEL"));
    assert!(approval.0.load(Ordering::Acquire));
}
