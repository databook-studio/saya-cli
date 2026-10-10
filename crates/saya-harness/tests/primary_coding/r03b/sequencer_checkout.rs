use std::{num::NonZeroU64, time::Duration};

use saya_harness::coding::budget::SequencerError;
use saya_store::StoreError;
use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1, GitObjectId,
    JobId, JobState, ManagedCheckoutId, ManagedCheckoutLifecycle, ManagedCheckoutPath,
    ManagedCheckoutRecordData, ManagedCheckoutRecordV1, RepositoryId, RepositoryMutationAction,
    Sha256Digest, SnapshotId,
};

use super::support::{Appends, Fixture, limits};

fn checkout_intent_event(
    job: &JobId,
    sequence: u64,
    checkout: &ManagedCheckoutId,
    repository: &RepositoryId,
    snapshot: &SnapshotId,
) -> CodingEventV1 {
    let record = ManagedCheckoutRecordV1::new(ManagedCheckoutRecordData {
        job_id: job.clone(),
        agent_id: None,
        managed_checkout_id: checkout.clone(),
        repository_id: repository.clone(),
        common_dir_sha256: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        managed_path: ManagedCheckoutPath::new("/worktrees/checkout-a").unwrap(),
        base_commit: GitObjectId::parse(&"b".repeat(40)).unwrap(),
        snapshot_id: snapshot.clone(),
        snapshot_sha256: Sha256Digest::parse(&"c".repeat(64)).unwrap(),
        created_by_sequence: NonZeroU64::new(sequence).unwrap(),
        lifecycle: ManagedCheckoutLifecycle::Creating,
    })
    .unwrap();
    let operation = CheckoutOperationV1::new(
        job.clone(),
        NonZeroU64::new(sequence).unwrap(),
        checkout.clone(),
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Intent,
    )
    .unwrap();
    CodingEventV1::new(
        NonZeroU64::new(sequence).unwrap(),
        job.clone(),
        None,
        None,
        CodingEventKind::CheckoutCreateIntent {
            checkout: Box::new(record),
            operation: Box::new(operation),
        },
    )
    .unwrap()
}

fn checkout_outcome_event(
    job: &JobId,
    sequence: u64,
    operation_sequence: u64,
    checkout: &ManagedCheckoutId,
) -> CodingEventV1 {
    let operation = CheckoutOperationV1::new(
        job.clone(),
        NonZeroU64::new(operation_sequence).unwrap(),
        checkout.clone(),
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Completed,
    )
    .unwrap();
    CodingEventV1::new(
        NonZeroU64::new(sequence).unwrap(),
        job.clone(),
        None,
        None,
        CodingEventKind::CheckoutOperationOutcome {
            operation: Box::new(operation),
        },
    )
    .unwrap()
}

#[tokio::test]
async fn checkout_intent_and_outcome_take_consecutive_sequences() {
    let fixture = Fixture::new("sequencer-checkout-flow").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let sequencer = ledger.sequencer();
    let appends = Appends {
        sequencer: &sequencer,
        job: &fixture.job,
    };
    let child = fixture
        .add_child("agent-checkout", "invocation-checkout")
        .await;
    let actor = (child.agent_id(), child.invocation_id());
    let _state = appends.child_state(actor, JobState::Running).await;
    let checkout = ManagedCheckoutId::parse("checkout-a").unwrap();
    let intent = sequencer
        .append_checkout_intent(|sequence| {
            checkout_intent_event(
                &fixture.job,
                sequence,
                &checkout,
                &fixture.repository,
                &fixture.snapshot,
            )
        })
        .await
        .unwrap();
    let intent_sequence = intent.sequence.get();
    let outcome = sequencer
        .append_checkout_outcome(|sequence| {
            checkout_outcome_event(&fixture.job, sequence, intent_sequence, &checkout)
        })
        .await
        .unwrap();
    assert_eq!(intent.sequence.get(), 3);
    assert_eq!(outcome.sequence.get(), intent.sequence.get() + 1);
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    let sequences: Vec<u64> = page
        .events
        .iter()
        .map(|event| event.sequence().get())
        .collect();
    assert_eq!(sequences, vec![1, 2, 3, 4]);
    assert!(matches!(
        page.events[2].event(),
        CodingEventKind::CheckoutCreateIntent { .. }
    ));
    assert!(matches!(
        page.events[3].event(),
        CodingEventKind::CheckoutOperationOutcome { .. }
    ));
    let snapshot = ledger.snapshot().await;
    assert!(!snapshot.poisoned);
    fixture.close().await;
}

#[tokio::test]
async fn checkout_intent_with_foreign_job_is_refused_without_poisoning() {
    let fixture = Fixture::new("sequencer-checkout-foreign").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let sequencer = ledger.sequencer();
    let appends = Appends {
        sequencer: &sequencer,
        job: &fixture.job,
    };
    let foreign = JobId::parse("job-b").unwrap();
    let checkout = ManagedCheckoutId::parse("checkout-a").unwrap();
    let error = sequencer
        .append_checkout_intent(|sequence| {
            checkout_intent_event(
                &foreign,
                sequence,
                &checkout,
                &fixture.repository,
                &fixture.snapshot,
            )
        })
        .await
        .unwrap_err();
    assert!(matches!(error, SequencerError::ForeignJob));
    let state = appends
        .job_state(JobState::WaitingForApproval)
        .await
        .unwrap();
    assert_eq!(state.sequence.get(), 2);
    let snapshot = ledger.snapshot().await;
    assert!(!snapshot.poisoned);
    fixture.close().await;
}

#[tokio::test]
async fn checkout_outcome_without_intent_poisons_owner() {
    let fixture = Fixture::new("sequencer-checkout-orphan-outcome").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let sequencer = ledger.sequencer();
    let appends = Appends {
        sequencer: &sequencer,
        job: &fixture.job,
    };
    let checkout = ManagedCheckoutId::parse("checkout-a").unwrap();
    let error = sequencer
        .append_checkout_outcome(|sequence| {
            checkout_outcome_event(&fixture.job, sequence, 1, &checkout)
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        SequencerError::Store(_, StoreError::Invalid)
    ));
    let next = appends.job_state(JobState::WaitingForApproval).await;
    assert!(matches!(next, Err(SequencerError::Poisoned)));
    let snapshot = ledger.snapshot().await;
    assert!(snapshot.poisoned);
    fixture.close().await;
}
