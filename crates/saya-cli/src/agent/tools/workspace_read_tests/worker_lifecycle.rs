use super::Sandbox;
use saya_agent::{CancellationReason, CancellationToken, ToolExecutionContext, ToolExecutor};

#[test]
fn abandoned_read_stays_owned_until_the_blocking_worker_drains() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let sandbox = Sandbox::new("read-abandoned-worker");
        sandbox.ws.write("entry.txt", b"read me").unwrap();
        let tools = sandbox.tools();
        let (release, held) = std::sync::mpsc::channel();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            held.recv().unwrap();
        });
        ready.await.unwrap();

        let cancellation = CancellationToken::new();
        let mut read = Box::pin(tools.execute_with_context(
            "read",
            serde_json::json!({"path": "entry.txt"}),
            ToolExecutionContext {
                result_cap: 4096,
                cancellation: cancellation.clone(),
            },
        ));
        let dispatched = matches!(futures_util::poll!(read.as_mut()), std::task::Poll::Pending);
        drop(read);
        let reason = cancellation.reason();
        let mut settle = Box::pin(tools.settle_invocation());
        let retained = matches!(
            futures_util::poll!(settle.as_mut()),
            std::task::Poll::Pending
        );

        release.send(()).unwrap();
        blocker.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(4), settle)
            .await
            .unwrap()
            .unwrap();
        assert!(dispatched, "the read must be moved off the async caller");
        assert_eq!(reason, Some(CancellationReason::OwnerFailure));
        assert!(
            retained,
            "settlement must wait for the queued worker to drain"
        );
        let result = tools
            .execute("read", serde_json::json!({"path": "entry.txt"}))
            .await
            .unwrap();
        assert_eq!(result["content"], "     1→read me");
    });
}
