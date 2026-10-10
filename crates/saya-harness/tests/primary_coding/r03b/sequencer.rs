use std::{sync::Arc, time::Duration};

use saya_types::{CodingEventKind, JobState};

use super::support::{Appends, CHILD_LIMITS, Fixture, limits};

#[tokio::test]
async fn sequencer_shared_writers_emit_exact_consecutive_sequences() {
    let fixture = Fixture::new("sequencer-shared-writers").await;
    let first = fixture.add_child("agent-seq-a", "invocation-seq-a").await;
    let second = fixture.add_child("agent-seq-b", "invocation-seq-b").await;
    let ledger = fixture
        .ledger(limits(8, 8, 8, 4, 4, Duration::from_secs(30)))
        .await
        .unwrap();
    let sequencer = ledger.sequencer();
    let appends = Appends {
        sequencer: &sequencer,
        job: &fixture.job,
    };
    let _first_session = ledger
        .begin_child(first.agent_id(), first.invocation_id(), CHILD_LIMITS)
        .await
        .unwrap();
    let first_actor = (first.agent_id(), first.invocation_id());
    let second_actor = (second.agent_id(), second.invocation_id());
    // The reservation consumes sequence 2; sequencer appends take 3 and 4,
    // a second reservation takes 5, and the sequencer resumes at 6.
    let state_a = appends.child_state(first_actor, JobState::Running).await;
    let reported = appends.child_result(first_actor).await;
    let _second_session = ledger
        .begin_child(second.agent_id(), second.invocation_id(), CHILD_LIMITS)
        .await
        .unwrap();
    let state_b = appends.child_state(second_actor, JobState::Running).await;
    assert_eq!(state_a.sequence.get(), 3);
    assert_eq!(reported.sequence.get(), 4);
    assert_eq!(state_b.sequence.get(), 6);
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    let events = page.events;
    let sequences: Vec<u64> = events.iter().map(|event| event.sequence().get()).collect();
    assert_eq!(sequences, vec![1, 2, 3, 4, 5, 6]);
    assert!(matches!(
        events[1].event(),
        CodingEventKind::BudgetReserved { .. }
    ));
    assert_eq!(
        events[2].event(),
        &CodingEventKind::StateChanged {
            state: JobState::Running
        }
    );
    assert!(matches!(
        events[3].event(),
        CodingEventKind::ResultReported { .. }
    ));
    assert!(matches!(
        events[4].event(),
        CodingEventKind::BudgetReserved { .. }
    ));
    // Sequencer events carry the child actor; the fixture's sequence 1 does not.
    assert!(events[2].agent_id().is_some());
    assert!(events[0].agent_id().is_none());
    let reservations = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 50)
        .await
        .unwrap();
    assert_eq!(reservations.items.len(), 2);
    assert_eq!(
        reservations.items[0]
            .reservation
            .reservation_sequence()
            .get(),
        2
    );
    assert_eq!(
        reservations.items[1]
            .reservation
            .reservation_sequence()
            .get(),
        5
    );
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.child_slots_active, 2);
    assert!(!snapshot.poisoned);
    fixture.close().await;
}

#[tokio::test]
async fn sequencer_view_is_cheap_and_shared() {
    let fixture = Fixture::new("sequencer-shared-view").await;
    let ledger = fixture
        .ledger(limits(64, 64, 64, 64, 64, Duration::from_secs(120)))
        .await
        .unwrap();
    let sequencer = ledger.sequencer();
    let mirrored = sequencer.clone();
    // All 16 tasks wait here so their first appends genuinely overlap
    // instead of racing only by scheduler timing.
    let barrier = Arc::new(tokio::sync::Barrier::new(16));
    let mut handles = Vec::new();
    for index in 0..16 {
        let child = fixture
            .add_child(
                &format!("agent-task-{index}"),
                &format!("invocation-task-{index}"),
            )
            .await;
        let view = if index % 2 == 0 {
            sequencer.clone()
        } else {
            mirrored.clone()
        };
        let job = fixture.job.clone();
        let agent = child.agent_id().clone();
        let invocation = child.invocation_id().clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let appends = Appends {
                sequencer: &view,
                job: &job,
            };
            let mut sequences = Vec::new();
            // Each child walks Queued -> Running -> WaitingForApproval ->
            // Uncertain and then reports its result: four store-accepted
            // events with no projection conflict.
            for state in [
                JobState::Running,
                JobState::WaitingForApproval,
                JobState::Uncertain,
            ] {
                let appended = appends.child_state((&agent, &invocation), state).await;
                sequences.push(appended.sequence.get());
            }
            let appended = appends.child_result((&agent, &invocation)).await;
            sequences.push(appended.sequence.get());
            sequences
        }));
    }
    let mut allocated = Vec::new();
    for handle in handles {
        allocated.extend(handle.await.unwrap());
    }
    allocated.sort_unstable();
    assert_eq!(allocated, (2..=65).collect::<Vec<u64>>());
    let page = fixture
        .store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    assert!(page.next_cursor.is_none());
    assert_eq!(page.events.len(), 65);
    for (index, event) in page.events.iter().enumerate() {
        assert_eq!(event.sequence().get(), u64::try_from(index).unwrap() + 1);
    }
    let snapshot = ledger.snapshot().await;
    assert!(!snapshot.poisoned);
    fixture.close().await;
}
