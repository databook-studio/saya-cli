use super::super::{event_projection_storage::event, token_charge_reservations::reservation};
use super::support::{Fixture, digest, nz};
use saya_store::{ChildInvocationError, CodingRuntimeAuthority, SqliteStateStore};
use saya_types::{ChildInvocationRecord, CodingEventKind, CodingEventV1, JobState};

fn adoption(f: &Fixture, prepared: &CodingEventV1, instruction: &str) -> CodingEventV1 {
    CodingEventV1::new(
        nz(f.next),
        f.base.job.clone(),
        Some(f.child.agent_id.clone()),
        prepared.invocation_id().cloned(),
        CodingEventKind::ChildInvocation {
            record: Box::new(ChildInvocationRecord::FollowUpAdopted {
                prepared_sequence: prepared.sequence(),
                source_sha256: digest("c"),
                instruction_sha256: digest(instruction),
            }),
        },
    )
    .unwrap()
}
async fn prepare(f: &mut Fixture) -> CodingEventV1 {
    let completion = f.complete().await;
    let prepared = f.prepared(completion);
    f.base
        .store
        .record_coding_child_invocation(prepared.clone(), &f.authority)
        .await
        .unwrap();
    f.child.current_invocation_id = prepared.invocation_id().unwrap().clone();
    f.next += 1;
    prepared
}
async fn renew(f: &mut Fixture) {
    let paused = event(
        f.next,
        &f.base.job,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Paused,
        },
    );
    f.base.store.append_coding_event(paused).await.unwrap();
    let fresh = CodingRuntimeAuthority {
        epoch: 2,
        policy_digest: digest("e"),
    };
    f.base
        .store
        .renew_coding_runtime(&f.base.job, Some(&f.authority), &fresh, f.next)
        .await
        .unwrap();
    f.next += 2;
    f.authority = fresh;
}
#[tokio::test]
async fn queued_preparation_requires_fresh_exact_adoption() {
    let mut f = Fixture::new("follow-adopt").await;
    let prepared = prepare(&mut f).await;
    let no_resume = adoption(&f, &prepared, "d");
    assert!(matches!(
        f.base
            .store
            .record_coding_child_invocation(no_resume, &f.authority)
            .await,
        Err(ChildInvocationError::Refused)
    ));
    f.base.store.close().await;
    f.base.store = SqliteStateStore::new(f.base.root.join("state.sqlite3"));
    renew(&mut f).await;
    let changed = adoption(&f, &prepared, "f");
    assert!(matches!(
        f.base
            .store
            .record_coding_child_invocation(changed, &f.authority)
            .await,
        Err(ChildInvocationError::Refused)
    ));
    let exact = adoption(&f, &prepared, "d");
    f.base
        .store
        .record_coding_child_invocation(exact, &f.authority)
        .await
        .unwrap();
    f.next += 1;
    let duplicate = adoption(&f, &prepared, "d");
    assert!(matches!(
        f.base
            .store
            .record_coding_child_invocation(duplicate, &f.authority)
            .await,
        Err(ChildInvocationError::Refused)
    ));
    let child = f
        .base
        .store
        .list_coding_children(&f.base.job, None, 100)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    assert_eq!(
        child.current_invocation_id,
        *prepared.invocation_id().unwrap()
    );
    assert_eq!(child.state, JobState::Queued);
    assert_eq!(
        f.base
            .store
            .coding_child_invocation_history(&f.base.job, &f.child.agent_id)
            .await
            .unwrap()
            .len(),
        2
    );
    f.base.close().await;
}
#[tokio::test]
async fn reserved_prepared_invocation_cannot_be_adopted() {
    let mut f = Fixture::new("follow-adopt-used").await;
    let prepared = prepare(&mut f).await;
    renew(&mut f).await;
    let reserved = reservation(&f.base.job, f.next, Some(&f.child));
    let event = f.event(CodingEventKind::BudgetReserved {
        reservation: Box::new(reserved),
    });
    let reserved_sequence = event.sequence();
    f.base
        .store
        .reserve_coding_budget_fenced(event, Some(&f.authority))
        .await
        .unwrap();
    let adopted = adoption(&f, &prepared, "d");
    assert!(matches!(
        f.base
            .store
            .record_coding_child_invocation(adopted, &f.authority)
            .await,
        Err(ChildInvocationError::Refused)
    ));
    assert!(
        f.base
            .store
            .get_coding_budget_reservation(&f.base.job, reserved_sequence)
            .await
            .unwrap()
            .is_some()
    );
    f.base.close().await;
}
#[tokio::test]
async fn crashed_root_permit_requires_reconciliation_before_prepared_adoption() {
    let mut f = Fixture::new("follow-adopt-root").await;
    let completion = f.complete().await;
    let reserved = reservation(&f.base.job, f.next, None);
    let root = event(
        f.next,
        &f.base.job,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reserved),
        },
    );
    let root_sequence = root.sequence();
    f.base
        .store
        .reserve_coding_budget_fenced(root, Some(&f.authority))
        .await
        .unwrap();
    f.next += 1;
    let prepared = f.prepared(completion);
    f.base
        .store
        .record_coding_child_invocation(prepared.clone(), &f.authority)
        .await
        .unwrap();
    f.child.current_invocation_id = prepared.invocation_id().unwrap().clone();
    f.next += 1;
    let fresh = CodingRuntimeAuthority {
        epoch: 2,
        policy_digest: digest("e"),
    };
    assert_eq!(
        f.base
            .store
            .renew_coding_runtime(&f.base.job, Some(&f.authority), &fresh, f.next - 1)
            .await
            .unwrap_err(),
        saya_store::StoreError::Conflict
    );
    let reconciled = event(
        f.next,
        &f.base.job,
        None,
        CodingEventKind::BudgetReconciled {
            reservation_sequence: root_sequence,
            note_sha256: digest("f"),
        },
    );
    f.base
        .store
        .reconcile_coding_budget(reconciled)
        .await
        .unwrap();
    f.next += 1;
    renew(&mut f).await;
    let exact = adoption(&f, &prepared, "d");
    f.base
        .store
        .record_coding_child_invocation(exact, &f.authority)
        .await
        .unwrap();
    let reservation = f
        .base
        .store
        .get_coding_budget_reservation(&f.base.job, root_sequence)
        .await
        .unwrap()
        .unwrap();
    assert!(reservation.reconciled);
    assert_eq!(reservation.reservation.charges()[0].reserved().get(), 1);
    f.base.close().await;
}
