use std::time::Duration;

use saya_agent::CancellationReason;
use saya_harness::coding::budget::SequencerError;
use saya_types::{CodingEventKind, JobId, JobState};

use super::support::{Appends, Fixture, job_event, limits};

#[tokio::test]
async fn sequencer_external_conflict_poisons_owner() {
    let fixture = Fixture::new("sequencer-external-conflict").await;
    let ledger = fixture
        .ledger(limits(4, 4, 4, 2, 2, Duration::from_secs(30)))
        .await
        .unwrap();
    let sequencer = ledger.sequencer();
    let appends = Appends {
        sequencer: &sequencer,
        job: &fixture.job,
    };
    // Pre-seed through the real public store append at the exact sequence the
    // sequencer would allocate next, from outside the ledger.
    let paused = CodingEventKind::StateChanged {
        state: JobState::Paused,
    };
    let seeded = job_event(&fixture.job, 2, paused);
    fixture.store.append_coding_event(seeded).await.unwrap();
    let result = appends.job_state(JobState::WaitingForApproval).await;
    assert!(matches!(result, Err(SequencerError::Conflict(2))));
    let snapshot = ledger.snapshot().await;
    assert!(snapshot.poisoned);
    assert_eq!(
        ledger.cancellation_token().reason(),
        Some(CancellationReason::OwnerFailure)
    );
    // No later append is attempted and the pre-seeded event stays untouched.
    let retried = appends.job_state(JobState::WaitingForApproval).await;
    assert!(matches!(retried, Err(SequencerError::Poisoned)));
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    assert!(page.next_cursor.is_none());
    assert_eq!(page.events.len(), 2);
    assert_eq!(
        page.events[1].event(),
        &CodingEventKind::StateChanged {
            state: JobState::Paused
        }
    );
    fixture.close().await;
}

// A caller bug detectable before any append is refused typed and leaves the
// owner usable: nothing was consumed, no store call was attempted, and the
// ledger's invariants are intact — the same class as an admission denial.
// Poisoning is reserved for failures observed at or after the append attempt
// (external conflicts, store failures), where a bug can no longer be told
// apart from divergence and the owner must fail closed like the reservation
// append path does.
#[tokio::test]
async fn sequencer_rejects_foreign_job_event_before_append() {
    let fixture = Fixture::new("sequencer-foreign-job").await;
    let ledger = fixture
        .ledger(limits(4, 4, 4, 2, 2, Duration::from_secs(30)))
        .await
        .unwrap();
    let sequencer = ledger.sequencer();
    let foreign = Appends {
        sequencer: &sequencer,
        job: &JobId::parse("job-foreign").unwrap(),
    };
    let result = foreign.job_state(JobState::Running).await;
    assert!(matches!(result, Err(SequencerError::ForeignJob)));
    let snapshot = ledger.snapshot().await;
    assert!(!snapshot.poisoned);
    assert_eq!(ledger.cancellation_token().reason(), None);
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    assert_eq!(page.events.len(), 1);
    let owned = Appends {
        sequencer: &sequencer,
        job: &fixture.job,
    };
    // The refusal consumed nothing: the next append still allocates 2.
    let appended = owned.job_state(JobState::WaitingForApproval).await.unwrap();
    assert_eq!(appended.sequence.get(), 2);
    fixture.close().await;
}

// A closure that fills a sequence other than the allocated one is refused
// typed before any store call, the same caller-error class as the
// foreign-job refusal: nothing is appended, nothing is consumed, and the
// owner stays usable.
#[tokio::test]
async fn sequencer_rejects_mismatched_sequence_before_any_append() {
    let fixture = Fixture::new("sequencer-sequence-mismatch").await;
    let ledger = fixture
        .ledger(limits(4, 4, 4, 2, 2, Duration::from_secs(30)))
        .await
        .unwrap();
    let sequencer = ledger.sequencer();
    let appends = Appends {
        sequencer: &sequencer,
        job: &fixture.job,
    };
    // Pre-seed through the real public store append at the exact sequence
    // the sequencer will allocate next, from outside the ledger.
    let paused = CodingEventKind::StateChanged {
        state: JobState::Paused,
    };
    let seeded = job_event(&fixture.job, 2, paused);
    fixture.store.append_coding_event(seeded).await.unwrap();
    // The closure fills the NEXT sequence instead of the allocated one, as
    // if an external writer had already claimed the allocated slot.
    let mismatched = appends
        .sequencer
        .append_event(|sequence| {
            job_event(
                appends.job,
                sequence + 1,
                CodingEventKind::StateChanged {
                    state: JobState::WaitingForApproval,
                },
            )
        })
        .await;
    assert!(matches!(mismatched, Err(SequencerError::SequenceMismatch)));
    let snapshot = ledger.snapshot().await;
    assert!(!snapshot.poisoned);
    assert_eq!(ledger.cancellation_token().reason(), None);
    // The refusal happened before any store call: no event exists at the
    // sequence the closure filled, and the pre-seeded event is untouched.
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    assert!(page.next_cursor.is_none());
    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].sequence().get(), 1);
    assert_eq!(page.events[1].sequence().get(), 2);
    assert_eq!(
        page.events[1].event(),
        &CodingEventKind::StateChanged {
            state: JobState::Paused
        }
    );
    fixture.close().await;
}
