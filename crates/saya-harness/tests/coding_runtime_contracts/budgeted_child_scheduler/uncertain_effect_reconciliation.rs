use super::{
    child_scheduler::{caps, ledger, start},
    fixture::Fixture,
    lifecycle_support::{self, Behavior},
    openai_server::{LocalOpenAi, TOOL_RESPONSE},
};
use saya_agent::{EffectAdmission, EffectAdmissionError, EffectOutcome, EffectSettlement};
use saya_harness::coding::coordinator::{ResumeError, resume_job};
use saya_store::CodingRuntimeAuthority;
use saya_types::Sha256Digest;
use std::time::Duration;

#[tokio::test]
async fn aborted_effect_requires_reconciliation_and_never_auto_replays() {
    let fixture = Fixture::new("r12-no-replay").await;
    let ledger = ledger(&fixture).await;
    let limits = ledger.limits().await;
    let mut server = LocalOpenAi::new().await;
    let (child, entered) = lifecycle_support::node(
        &fixture,
        &ledger,
        &server,
        "agent-a",
        Behavior::IgnoreCancellation,
    )
    .await;
    let running = start(vec![child], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(TOOL_RESPONSE);
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    running.task.abort();
    assert!(running.task.await.unwrap_err().is_cancelled());
    running.progress.await.unwrap();
    let reopened = saya_store::SqliteStateStore::new(&fixture.database);
    let resumed = resume_job(
        reopened.clone(),
        fixture.job.clone(),
        limits,
        CodingRuntimeAuthority {
            epoch: 2,
            policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        },
    )
    .await;
    assert!(
        matches!(resumed, Err(ResumeError::ReconciliationRequired(status)) if status.unresolved_reservations > 0 && status.active_children == 1)
    );
    assert_eq!(server.count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        reopened
            .list_coding_completions(&fixture.job, None, 32)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    reopened.close().await;
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn exclusive_quiescence_refuses_all_admissions_until_guard_drops() {
    let fixture = Fixture::new("quiescence").await;
    let ledger = ledger(&fixture).await;
    let guard = ledger.try_quiescent().await.unwrap();
    assert_eq!(guard.job_id(), &fixture.job);
    assert!(matches!(
        ledger.begin_command().await,
        Err(EffectAdmissionError::Denied)
    ));
    let root = ledger.root_view();
    assert!(matches!(
        root.admit_model_request().await,
        Err(EffectAdmissionError::Denied)
    ));
    drop(guard);
    let permit = root.admit_model_request().await.unwrap();
    root.settle(
        permit,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::NotStarted,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
    assert!(ledger.try_quiescent().await.is_ok());
    let command = ledger.begin_command().await.unwrap();
    ledger
        .finish_command(command, EffectOutcome::Unknown)
        .await
        .unwrap();
    assert!(matches!(
        ledger.try_quiescent().await,
        Err(EffectAdmissionError::Denied)
    ));
    fixture.close().await;
}
