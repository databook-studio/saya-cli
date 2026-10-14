use super::support::Fixture;
use saya_store::{CodingEventAppend, StoreError};
use std::num::NonZeroU64;

#[tokio::test]
async fn a_checkout_intent_cannot_skip_the_job_event_sequence() {
    let fixture = Fixture::new("sequence-gap").await;
    let skipped = fixture.create_event(
        3,
        "checkout-r14-skipped",
        fixture.repository.clone(),
        fixture.snapshot.clone(),
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_create_intent(skipped)
            .await
            .unwrap_err(),
        StoreError::Conflict,
    );
    assert_eq!(
        fixture
            .store
            .list_coding_events(&fixture.job, None, 100)
            .await
            .unwrap()
            .events
            .len(),
        1,
    );
    let expected_next = fixture.create_event(
        2,
        "checkout-r14-next",
        fixture.repository.clone(),
        fixture.snapshot.clone(),
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_create_intent(expected_next)
            .await
            .unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(2).unwrap()),
    );
    fixture.close().await;
}
