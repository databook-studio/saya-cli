use super::{CodingMessage, deliver_outcome};

#[test]
fn outcome_is_delivered_even_when_the_activity_queue_is_full() {
    let (activity, activity_rx) = std::sync::mpsc::sync_channel::<CodingMessage>(2);
    let (receipts, receipts_rx) = std::sync::mpsc::sync_channel::<CodingMessage>(2);
    assert!(
        activity
            .try_send(CodingMessage::Activity("progress".into()))
            .is_ok()
    );
    assert!(
        activity
            .try_send(CodingMessage::Activity("progress".into()))
            .is_ok()
    );
    assert!(
        activity
            .try_send(CodingMessage::Activity("overflow".into()))
            .is_err()
    );
    deliver_outcome(&receipts, "receipt-text".into());
    // A full activity queue must not hold a control receipt: receipts ride
    // their own queue, so the outcome is available immediately and the
    // panel's next drain pushes it after the activity backlog.
    assert!(activity_rx.try_recv().is_ok());
    assert!(matches!(
        receipts_rx.try_recv(),
        Ok(CodingMessage::Activity(text)) if text == "receipt-text"
    ));
    assert!(receipts_rx.try_recv().is_err());
    drop(activity_rx);
    drop(receipts_rx);
}
