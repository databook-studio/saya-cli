use super::support::{Fixture, digest, nz};
use saya_store::{ChildInvocationError, SqliteStateStore};
use saya_types::{ChildInvocationRecord, CodingEventKind, CodingEventV1, InvocationId, JobState};

#[tokio::test]
async fn legacy_completed_child_has_no_inferred_scope_grant() {
    let mut f = Fixture::legacy("follow-legacy").await;
    let completion = f.complete().await;
    let event = f.prepared(completion);
    f.assert_refused(event).await;
    f.base.close().await;
}
#[tokio::test]
async fn stale_completion_and_cancelled_runtime_refuse_without_rotation() {
    let mut f = Fixture::new("follow-fences").await;
    let completion = f.complete().await;
    let wrong = f.prepared(nz(completion.get() - 1));
    f.assert_refused(wrong).await;
    f.base
        .store
        .request_coding_cancel(&f.base.job)
        .await
        .unwrap();
    let correct = f.prepared(completion);
    f.assert_refused(correct).await;
    f.base.close().await;
}
#[tokio::test]
async fn competing_process_connections_prepare_only_one_invocation() {
    let mut f = Fixture::new("follow-cas").await;
    let completion = f.complete().await;
    let first = f.prepared(completion);
    let second = CodingEventV1::new(
        first.sequence(),
        f.base.job.clone(),
        Some(f.child.agent_id.clone()),
        Some(InvocationId::parse("inv-racing-other").unwrap()),
        first.event().clone(),
    )
    .unwrap();
    let other = SqliteStateStore::new(f.base.root.join("state.sqlite3"));
    let (a, b) = tokio::join!(
        f.base
            .store
            .record_coding_child_invocation(first, &f.authority),
        other.record_coding_child_invocation(second, &f.authority)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(
        (&a, &b),
        (Ok(_), Err(ChildInvocationError::Refused)) | (Err(ChildInvocationError::Refused), Ok(_))
    ));
    assert_eq!(
        f.base
            .store
            .coding_child_invocation_history(&f.base.job, &f.child.agent_id)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        f.base
            .store
            .get_coding_completion(&f.base.job, completion)
            .await
            .unwrap()
            .is_some()
    );
    other.close().await;
    f.base.close().await;
}
#[tokio::test]
async fn follow_up_limit_survives_completed_invocations() {
    let mut f = Fixture::new("follow-limit").await;
    for index in 1..=3 {
        let completion = f.complete().await;
        let event = f.prepared_index(completion, index);
        f.rotate_and_start(event).await;
    }
    let completion = f.complete().await;
    let old = f.prepared_index(completion, 3);
    let fourth = CodingEventV1::new(
        old.sequence(),
        f.base.job.clone(),
        Some(f.child.agent_id.clone()),
        Some(InvocationId::parse("inv-follow-up-fourth").unwrap()),
        old.event().clone(),
    )
    .unwrap();
    f.assert_refused(fourth).await;
    assert_eq!(
        f.base
            .store
            .coding_child_invocation_history(&f.base.job, &f.child.agent_id)
            .await
            .unwrap()
            .len(),
        4
    );
    f.base.close().await;
}
#[tokio::test]
async fn scope_digest_mismatch_refuses_same_checkout_rotation() {
    let mut f = Fixture::new("follow-scope").await;
    let completion = f.complete().await;
    let old = f.prepared(completion);
    let CodingEventKind::ChildInvocation { record } = old.event() else {
        panic!("typed continuation");
    };
    let mut changed = record.as_ref().clone();
    if let ChildInvocationRecord::FollowUpPrepared { scope_sha256, .. } = &mut changed {
        *scope_sha256 = digest("e");
    }
    let event = CodingEventV1::new(
        old.sequence(),
        f.base.job.clone(),
        Some(f.child.agent_id.clone()),
        old.invocation_id().cloned(),
        CodingEventKind::ChildInvocation {
            record: Box::new(changed),
        },
    )
    .unwrap();
    f.assert_refused(event).await;
    assert_eq!(
        f.base
            .store
            .list_coding_children(&f.base.job, None, 100)
            .await
            .unwrap()
            .items[0]
            .state,
        JobState::Completed
    );
    f.base.close().await;
}
