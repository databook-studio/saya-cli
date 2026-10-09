#[tokio::test]
async fn source_cancellation_keeps_the_typed_replay_future_until_settlement() {
    use saya_agent::CancellationToken;
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };
    use tokio::sync::oneshot;

    let source = CancellationToken::new();
    let operation_control = CancellationToken::new();
    let (started_tx, started_rx) = oneshot::channel();
    let (cancel_seen_tx, cancel_seen_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let operation_settled = std::sync::Arc::new(AtomicBool::new(false));
    let operation = {
        let operation_control = operation_control.clone();
        let operation_settled = std::sync::Arc::clone(&operation_settled);
        async move {
            let _ = started_tx.send(());
            operation_control.cancelled().await;
            let _ = cancel_seen_tx.send(());
            let _ = release_rx.await;
            operation_settled.store(true, Ordering::Release);
            "late typed replay result"
        }
    };
    let task = tokio::spawn({
        let source = source.clone();
        let operation_control = operation_control.clone();
        async move {
            retain_replay(
                operation,
                &operation_control,
                source.cancelled(),
                || source.is_cancelled(),
                tokio::time::Instant::now() + Duration::from_secs(10),
            )
            .await
        }
    });

    started_rx.await.unwrap();
    source.cancel();
    cancel_seen_rx.await.unwrap();
    assert!(
        !task.is_finished(),
        "the stop does not drop the replay future"
    );
    assert!(!operation_settled.load(Ordering::Acquire));
    release_tx.send(()).unwrap();

    assert!(matches!(
        task.await.unwrap(),
        ReplayCompletion::Cancelled(Some("late typed replay result"))
    ));
    assert!(operation_settled.load(Ordering::Acquire));
}

#[tokio::test]
async fn visible_replay_stop_and_expired_deadline_do_not_poll_the_operation() {
    use saya_agent::CancellationToken;
    use std::{
        future::{pending, poll_fn},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::Poll,
        time::Duration,
    };

    let polls = Arc::new(AtomicUsize::new(0));
    let source = CancellationToken::new();
    source.cancel();
    let observed = Arc::clone(&polls);
    let ready_operation = poll_fn(move |_| {
        observed.fetch_add(1, Ordering::Relaxed);
        Poll::Ready(())
    });
    assert!(matches!(
        retain_replay(
            ready_operation,
            &CancellationToken::new(),
            source.cancelled(),
            || source.is_cancelled(),
            tokio::time::Instant::now() + Duration::from_secs(10),
        )
        .await,
        ReplayCompletion::Cancelled(None)
    ));
    assert_eq!(polls.load(Ordering::Relaxed), 0);

    let observed = Arc::clone(&polls);
    let expired_operation = poll_fn(move |_| {
        observed.fetch_add(1, Ordering::Relaxed);
        Poll::Ready(())
    });
    assert!(matches!(
        retain_replay(
            expired_operation,
            &CancellationToken::new(),
            pending(),
            || false,
            tokio::time::Instant::now() - Duration::from_millis(1),
        )
        .await,
        ReplayCompletion::TimedOut(None)
    ));
    assert_eq!(polls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn cancelled_replay_slot_waiter_leaves_the_holder_owned() {
    use std::{
        future::{Future, poll_fn},
        pin::Pin,
        task::Poll,
        time::Duration,
    };
    use tokio::sync::{Mutex, oneshot};

    let replay_slot = Mutex::new(());
    let holder = replay_slot.lock().await;
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let mut waiter = Box::pin(acquire_replay_slot(
        &replay_slot,
        async move {
            let _ = cancel_rx.await;
        },
        || false,
        tokio::time::Instant::now() + Duration::from_secs(10),
    ));

    poll_fn(|cx| match Pin::as_mut(&mut waiter).poll(cx) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(_) => panic!("the holder keeps the replay slot locked"),
    })
    .await;
    cancel_tx.send(()).unwrap();
    assert!(matches!(waiter.await, Err(ReplayStop::Cancelled)));
    assert!(
        replay_slot.try_lock().is_err(),
        "the original holder remains"
    );
    drop(holder);
    assert!(
        replay_slot.try_lock().is_ok(),
        "the original holder releases it"
    );
}
