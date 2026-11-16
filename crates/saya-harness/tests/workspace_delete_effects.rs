//! Landed effects survive cancellation and failed authoritative settlement.
#![cfg(unix)]
#[path = "workspace_delete_effects/support.rs"]
mod support;
use saya_agent::{
    AgentError, AgentLimits, AgentRunControl, AllowReadOnlyApproval, AnsweringLimits,
    CancellationReason, CancellationToken, EffectOutcome, run_agent_with_control,
};
use saya_harness::workspace::Workspace;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use support::*;

// This actual filesystem executor feeds the public loop's real settlement
// boundary. The owner records attempts only; compiled-CLI SQLite fault tests
// independently own durable unknown-accounting proof.
#[tokio::test]
async fn landed_deletion_stays_accounted_when_cancelled_or_settlement_fails() {
    for fail in [false, true] {
        let root = Root::new();
        let workspace = Workspace::open(&root.0).unwrap();
        let digest = workspace.read("victim.rs", 0).unwrap().digest;
        let provider = Provider {
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            digest,
        };
        let owner = Owner::new(root.0.join("victim.rs"), fail);
        let token = CancellationToken::new();
        let sink = Sink {
            token: token.clone(),
            target: owner.target.clone(),
            cancel: !fail,
        };
        let control = AgentRunControl::new(
            AgentLimits {
                permit_workspace_writes: true,
                ..AgentLimits::default()
            },
            AnsweringLimits::default(),
            &AllowReadOnlyApproval,
            &sink,
            token.clone(),
        )
        .with_effect_admission(&owner);
        let result = run_agent_with_control(
            &provider,
            &Files(workspace),
            request(),
            definitions(),
            control,
        )
        .await;
        if fail {
            assert!(matches!(result, Err(AgentError::EffectOwnerFailure(_))));
            assert_eq!(token.reason(), Some(CancellationReason::OwnerFailure));
        } else {
            assert!(matches!(result, Err(AgentError::Cancelled)));
            assert_eq!(token.reason(), Some(CancellationReason::User));
        }
        assert!(!owner.target.exists());
        assert_eq!(
            *owner.tools.lock().unwrap(),
            vec![EffectOutcome::Succeeded],
            "a landed unlink is never refunded as NotStarted"
        );
        assert_eq!(
            provider.calls.load(Ordering::Acquire),
            1,
            "no later dispatch or false success answer"
        );
    }
}
