use async_trait::async_trait;
use saya_agent::{
    AgentRunControl, AnsweringLimits, CancellationToken, EffectAdmission, EffectAdmissionError,
    EffectOutcome, EffectOwnerFailure, EffectPermit, EffectSettlement, ToolCall, ToolEffect,
    run_agent_with_control,
};
use saya_harness::coding::policy::ChildApprovalDecider;
use std::{
    num::NonZeroU64,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::sync::Mutex;

use super::super::{
    child_toolset,
    tests::{Sandbox, checkout, implementer_view},
};
use super::{NoopSink, ScriptedProvider, limits, request};

#[derive(Default)]
struct RecordingEffects {
    next: AtomicU64,
    settlements: Mutex<Vec<EffectSettlement>>,
}

impl RecordingEffects {
    fn permit(&self) -> EffectPermit {
        EffectPermit::new(
            NonZeroU64::new(self.next.fetch_add(1, Ordering::Relaxed) + 1)
                .expect("permit ids start above zero"),
        )
    }
}

#[async_trait]
impl EffectAdmission for RecordingEffects {
    async fn admit_model_request(&self) -> Result<EffectPermit, EffectAdmissionError> {
        Ok(self.permit())
    }

    async fn admit_tool_batch(
        &self,
        effects: &[ToolEffect],
    ) -> Result<Vec<EffectPermit>, EffectAdmissionError> {
        Ok(effects.iter().map(|_| self.permit()).collect())
    }

    async fn settle(
        &self,
        _: EffectPermit,
        settlement: EffectSettlement,
    ) -> Result<(), EffectOwnerFailure> {
        self.settlements.lock().await.push(settlement);
        Ok(())
    }
}

#[tokio::test]
async fn child_multi_edit_overlap_dispatch_settles_as_known_no_effect() {
    let sandbox = Sandbox::new("loop-multi-overlap");
    let original = b"abc";
    std::fs::create_dir_all(sandbox.ws_root().join("a")).unwrap();
    std::fs::write(sandbox.ws_root().join("a/allowed.txt"), original).unwrap();
    let digest = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(original))
    };
    let checkout = checkout("child-loop");
    let view = implementer_view(&checkout);
    let (definitions, tools) = child_toolset(&view, &sandbox.pair(&checkout)).unwrap();
    let approval = ChildApprovalDecider::from_view(&view);
    let owner = RecordingEffects::default();
    let control = AgentRunControl::new(
        limits(&view),
        AnsweringLimits {
            max_requests: Some(2),
            max_tool_calls: Some(2),
            max_reported_tokens: Some(1024),
        },
        &approval,
        &NoopSink,
        CancellationToken::new(),
    )
    .with_effect_admission(&owner);
    let provider = ScriptedProvider {
        call: ToolCall {
            id: "edit-overlap".into(),
            name: "edit".into(),
            arguments: serde_json::json!({
                "path": "a/allowed.txt",
                "expected_digest": digest,
                "edits": [
                    {"old_text": "ab", "new_text": "X"},
                    {"old_text": "bc", "new_text": "Y"}
                ]
            }),
        },
        turn: std::sync::Mutex::new(0),
    };

    let output = run_agent_with_control(&provider, &tools, request(), definitions, control)
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(sandbox.ws_root().join("a/allowed.txt")).unwrap(),
        original
    );
    assert_eq!(output.tool_metadata[0].status, "failed");
    assert!(owner.settlements.lock().await.iter().any(|settlement| {
        matches!(
            settlement,
            EffectSettlement::Tool {
                outcome: EffectOutcome::Succeeded
            }
        )
    }));
}
