/// Drives the joined operation until it is observed pending — it owns no
/// settled result yet, so any later samples must come from a later phase.
async fn assert_pending(
    waiter: &mut tokio::task::JoinHandle<Result<serde_json::Value, ToolError>>,
    label: &str,
) {
    poll_fn(|cx| match Pin::new(&mut *waiter).poll(cx) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(result) => panic!("{label}: operation settled before the lease: {result:?}"),
    })
    .await;
}

async fn wait_for_executions(executions: &AtomicUsize, expected: usize) {
    for _ in 0..2000 {
        if executions.load(Ordering::SeqCst) == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("operation never reached {expected} connector executions");
}
