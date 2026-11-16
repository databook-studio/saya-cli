use super::retry_tests::{CountingAdmission, permit};
use super::*;
use crate::EffectCallKind;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// A cancellation during establishment must leave the admitted attempt for the
/// loop to settle. The loop can drop this provider future at any await, so a
/// settlement performed here would race that drop and orphan the permit, which
/// then poisons the whole budget owner.
#[tokio::test]
async fn cancelled_establishment_leaves_the_attempt_for_the_loop() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicBool::new(false));
    let opened = Arc::clone(&accepted);
    let server = tokio::spawn(async move {
        if let Ok((socket, _)) = listener.accept().await {
            let mut buffer = [0_u8; 1024];
            let _ = socket.readable().await;
            let _ = socket.try_read(&mut buffer);
            opened.store(true, Ordering::SeqCst);
            // Hold the response open until the test aborts this task.
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
    let admission = CountingAdmission::default();
    let cancellation = CancellationToken::new();
    let mut control = PhysicalAttemptControl::new(
        &admission,
        &cancellation,
        permit(99),
        EffectCallKind::Answer,
        None,
    );
    let client = reqwest::Client::new();
    let url = format!("http://{addr}");
    let mut send = Box::pin(send_stream_controlled(
        || client.get(&url),
        RetryPolicy::new(&[]),
        &cancellation,
        &url,
        Duration::from_secs(30),
        &mut control,
    ));
    tokio::select! {
        () = async {
            while !accepted.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        } => {}
        result = &mut send => panic!("send ended before the request opened: {result:?}"),
    }
    cancellation.cancel();
    let error = send.as_mut().await.unwrap_err();
    drop(send);
    assert!(
        matches!(
            error,
            ControlledProviderError::Provider(ProviderError::Cancelled)
        ),
        "{error:?}"
    );
    assert_eq!(
        admission.settled.load(Ordering::SeqCst),
        0,
        "the provider must not settle an establishment the loop may drop"
    );
    control.settle_aborted_establishment().await.unwrap();
    assert_eq!(
        admission.settled.load(Ordering::SeqCst),
        1,
        "the loop settles the abandoned attempt exactly once"
    );
    assert_eq!(
        admission.unknown_settlements.load(Ordering::SeqCst),
        1,
        "a dispatched send is settled unknown"
    );
    server.abort();
}
