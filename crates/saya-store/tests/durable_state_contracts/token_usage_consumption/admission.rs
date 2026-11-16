use super::fixture::*;
use saya_store::StoreError;
use saya_types::{CodingEventKind, JobState};
use std::num::NonZeroU64;

async fn rejected(label: &str, spec: RequestSpec<'_>, state: Option<JobState>, stale_child: bool) {
    let (path, store, job) = running(label).await;
    let actor = child(&store, &job, label).await;
    let req = request(
        &job,
        2,
        Some((&actor.0, &actor.1)),
        RequestSpec {
            host: "example.com",
            digest: "a",
            epoch: 7,
            ..spec
        },
    );
    store
        .record_coding_approval_request(request_event(&req))
        .await
        .unwrap();
    if stale_child {
        let raw = sqlx::SqlitePool::connect(&format!(
            "sqlite://{}",
            path.join("state.sqlite3").display()
        ))
        .await
        .unwrap();
        sqlx::query("UPDATE coding_children SET current_invocation_id='invocation-r13b-replaced' WHERE job_id=? AND agent_id=?").bind(job.as_str()).bind(actor.0.as_str()).execute(&raw).await.unwrap();
        raw.close().await;
    }
    if let Some(state) = state {
        store
            .append_coding_event(event(
                3,
                &job,
                None,
                CodingEventKind::StateChanged { state },
            ))
            .await
            .unwrap();
    }
    let sequence = if state.is_some() { 4 } else { 3 };
    let mismatched = request(&job, 2, Some((&actor.0, &actor.1)), spec);
    let context = saya_store::CodingApprovalConsumptionContext::new(
        mismatched.policy_sha256().clone(),
        mismatched.revocation_epoch(),
        mismatched.scope().clone(),
    );
    assert_eq!(
        store
            .consume_coding_approval(
                reservation(&job, sequence, Some((&actor.0, &actor.1)), 1),
                closure(&job, 2, sequence, Some((&actor.0, &actor.1))),
                context
            )
            .await
            .unwrap_err(),
        StoreError::Conflict,
        "{label}"
    );
    assert!(
        store
            .get_coding_approval_request(&job, NonZeroU64::new(2).unwrap())
            .await
            .unwrap()
            .unwrap()
            .closure
            .is_none(),
        "{label}"
    );
    store.close().await;
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn stale_policy_epoch_full_scope_child_invocation_and_time_refuse_pair() {
    let now = now();
    let defaults = |digest, epoch, requested, expires, host| RequestSpec {
        digest,
        epoch,
        requested,
        expires,
        host,
    };
    rejected(
        "digest",
        defaults("b", 7, now - 1, now + 60_000, "example.com"),
        None,
        false,
    )
    .await;
    rejected(
        "epoch",
        defaults("a", 8, now - 1, now + 60_000, "example.com"),
        None,
        false,
    )
    .await;
    rejected(
        "scope",
        defaults("a", 7, now - 1, now + 60_000, "other.example"),
        None,
        false,
    )
    .await;
    rejected(
        "expired",
        defaults(
            "a",
            7,
            now.saturating_sub(60_000),
            now.saturating_sub(1),
            "example.com",
        ),
        None,
        false,
    )
    .await;
    rejected(
        "future",
        defaults("a", 7, now + 60_000, now + 120_000, "example.com"),
        None,
        false,
    )
    .await;
    rejected(
        "stale-invocation",
        defaults("a", 7, now - 1, now + 60_000, "example.com"),
        None,
        true,
    )
    .await;
}

#[tokio::test]
async fn terminal_and_paused_job_refuse_consumption_before_events_commit() {
    let now = now();
    let spec = |requested, expires| RequestSpec {
        digest: "a",
        epoch: 7,
        requested,
        expires,
        host: "example.com",
    };
    rejected(
        "completed",
        spec(now - 1, now + 60_000),
        Some(JobState::Completed),
        false,
    )
    .await;
    rejected(
        "paused",
        spec(now - 1, now + 60_000),
        Some(JobState::Paused),
        false,
    )
    .await;
}

#[tokio::test]
async fn stale_actor_pair_refuses_without_closing_request() {
    let (path, store, job) = running("stale-actor").await;
    let owner = child(&store, &job, "approval-owner").await;
    let other = child(&store, &job, "different-actor").await;
    let req = append_request(
        &store,
        &job,
        2,
        Some((&owner.0, &owner.1)),
        now().saturating_sub(1),
        now() + 60_000,
    )
    .await;
    let result = store
        .consume_coding_approval(
            reservation(&job, 3, Some((&owner.0, &owner.1)), 1),
            closure(&job, 2, 3, Some((&other.0, &other.1))),
            context(&req),
        )
        .await;
    assert_eq!(result.unwrap_err(), StoreError::Conflict);
    assert!(
        store
            .get_coding_approval_request(&job, req.request_sequence())
            .await
            .unwrap()
            .unwrap()
            .closure
            .is_none()
    );
    assert_eq!(
        store
            .list_coding_events(&job, None, 10)
            .await
            .unwrap()
            .events
            .len(),
        2
    );
    store.close().await;
    let _ = std::fs::remove_dir_all(path);
}
