//! The resume path's finite history bound at the owner boundary. The ledger's
//! own caps (10,000 reservations per job) keep legal histories far below the
//! bound, so the history here is made by writing event rows straight into the
//! job's durable stream, the way a foreign or damaged store would present it:
//! resume must refuse the 65,537th event with a typed error rather than walk
//! the history without bound, and must still resume 65,536 events.

use std::time::Duration;

use saya_harness::coding::budget::{BudgetLedgerError, JobBudgetLedger};
use saya_types::{CodingEventKind, CodingEventV1, JobState};
use sqlx::{Connection, SqliteConnection};

use super::support::{Fixture, limits};

/// Appends running-state rows for every sequence after the stored maximum up
/// to and including `last`, bypassing the store's transition projection.
async fn pad_events_to(fixture: &Fixture, last: u64) {
    let url = format!("sqlite://{}", fixture.root.join("state.sqlite3").display());
    let mut connection = SqliteConnection::connect(&url).await.unwrap();
    let stored: i64 = sqlx::query_scalar("SELECT MAX(sequence) FROM coding_events WHERE job_id=?")
        .bind(fixture.job.as_str())
        .fetch_one(&mut connection)
        .await
        .unwrap();
    let mut transaction = connection.begin().await.unwrap();
    for sequence in (u64::try_from(stored).unwrap() + 1)..=last {
        let event = CodingEventV1::new(
            std::num::NonZeroU64::new(sequence).unwrap(),
            fixture.job.clone(),
            None,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        )
        .unwrap();
        let json = String::from_utf8(event.to_json_bytes().unwrap()).unwrap();
        sqlx::query("INSERT INTO coding_events(job_id, sequence, event_json) VALUES (?, ?, ?)")
            .bind(fixture.job.as_str())
            .bind(i64::try_from(sequence).unwrap())
            .bind(json)
            .execute(&mut *transaction)
            .await
            .unwrap();
    }
    transaction.commit().await.unwrap();
    connection.close().await.unwrap();
}

#[tokio::test]
async fn resume_accepts_65536_events_and_refuses_the_65537th() {
    let fixture = Fixture::new("resume-event-bound").await;
    let owner_limits = limits(8, 8, 8, 2, 2, Duration::from_secs(600));
    drop(fixture.ledger(owner_limits).await.unwrap());
    fixture.close_store().await;

    pad_events_to(&fixture, 65_536).await;
    let store = fixture.reopen();
    let resumed = JobBudgetLedger::resume(store.clone(), fixture.job.clone(), owner_limits)
        .await
        .expect("a history of exactly 65,536 events is within the resume bound");
    drop(resumed);
    fixture.close_store().await;

    pad_events_to(&fixture, 65_537).await;
    let store = fixture.reopen();
    let refused = JobBudgetLedger::resume(store.clone(), fixture.job.clone(), owner_limits).await;
    assert!(
        matches!(refused, Err(BudgetLedgerError::ResumeHistoryTooLong)),
        "the 65,537th event must be refused, got {:?}",
        refused.map(|_| ())
    );
    fixture.close().await;
}
