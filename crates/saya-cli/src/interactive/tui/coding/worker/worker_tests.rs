use super::{CodingMessage, deliver_outcome};

#[test]
fn outcome_is_delivered_even_when_the_activity_queue_is_full() {
    let (tx, rx) = std::sync::mpsc::sync_channel::<CodingMessage>(2);
    assert!(
        tx.try_send(CodingMessage::Activity("progress".into()))
            .is_ok()
    );
    assert!(
        tx.try_send(CodingMessage::Activity("progress".into()))
            .is_ok()
    );
    assert!(
        tx.try_send(CodingMessage::Activity("overflow".into()))
            .is_err()
    );
    let drained = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut seen = Vec::new();
        loop {
            match rx.recv() {
                Ok(CodingMessage::Activity(text)) => {
                    seen.push(text);
                    if seen.last().map(String::as_str) == Some("receipt-text") {
                        break seen;
                    }
                }
                Ok(CodingMessage::Approval { .. }) | Err(_) => break seen,
            }
        }
    });
    deliver_outcome(&tx, "receipt-text".into());
    let seen = drained.join().expect("drain thread must not panic");
    assert_eq!(seen.first().map(String::as_str), Some("progress"));
    assert_eq!(seen.get(1).map(String::as_str), Some("progress"));
    assert_eq!(seen.last().map(String::as_str), Some("receipt-text"));
    assert_eq!(seen.len(), 3);
}
