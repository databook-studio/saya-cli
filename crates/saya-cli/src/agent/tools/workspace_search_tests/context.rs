//! Real ToolExecutor context and actual blocking-pool lifetime, no product hooks.
use super::Sandbox;
use saya_agent::{
    CancellationReason, CancellationToken, ToolError, ToolExecutionContext, ToolExecutor,
};
use serde_json::json;

#[tokio::test]
async fn search_context_preserves_cancellation_and_the_exact_json_cap() {
    let sandbox = Sandbox::new("search-real-context");
    sandbox
        .ws
        .write("lines.txt", "needle\n".repeat(80).as_bytes())
        .unwrap();
    let tools = sandbox.tools();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let result = tools
        .execute_with_context(
            "grep",
            json!({"pattern":"needle"}),
            ToolExecutionContext {
                result_cap: 4096,
                cancellation: cancelled.clone(),
            },
        )
        .await;
    assert!(matches!(result, Err(ToolError::NoEffect(_))));
    assert_eq!(cancelled.reason(), Some(CancellationReason::User));
    let tiny = tools
        .execute_with_context(
            "grep",
            json!({"pattern":"needle"}),
            ToolExecutionContext {
                result_cap: 128,
                cancellation: CancellationToken::new(),
            },
        )
        .await;
    assert!(matches!(tiny, Err(ToolError::NoEffect(_))));
    let result = tools
        .execute_with_context(
            "grep",
            json!({"pattern":"needle"}),
            ToolExecutionContext {
                result_cap: 1024,
                cancellation: CancellationToken::new(),
            },
        )
        .await
        .unwrap();
    assert!(serde_json::to_vec(&result).unwrap().len() <= 1024);
    assert_eq!(result["total_matches"], 80);
    assert_eq!(result["matches_found"], 80);
    assert_eq!(result["coverage_complete"], true);
    assert_eq!(result["truncated"], true);
    assert!(!result["matches"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn search_adapter_delivers_scope_include_and_literal_or_regex_semantics() {
    let sandbox = Sandbox::new("search-adapter-options");
    for (path, bytes) in [
        (".gitignore", "*.tmp\n"),
        ("src/ok.rs", "a123\nliteral[0-9]+\n"),
        ("src/drop.tmp", "a123\n"),
        ("other/out.rs", "a123\n"),
    ] {
        sandbox.ws.write(path, bytes.as_bytes()).unwrap();
    }
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "grep",
            json!({"pattern":"^a[0-9]+$","regex":true,"path":"src","include":"*.rs"}),
        )
        .await
        .unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["matches"][0]["path"], "src/ok.rs");
    assert_eq!(result["matches"][0]["line"], 1);
    assert_eq!(result["total_matches"], 1);
    let literal = tools
        .execute(
            "grep",
            json!({"pattern":"literal[0-9]+","path":"src/ok.rs"}),
        )
        .await
        .unwrap();
    assert_eq!(literal["matches"][0]["line"], 2);
    assert_eq!(literal["matches"][0]["text"], "literal[0-9]+");
}

#[test]
fn abandoned_dispatched_search_stays_owned_until_actual_worker_drain() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let sandbox = Sandbox::new("search-abandoned-worker");
        sandbox.ws.write("entry.txt", b"needle\n").unwrap();
        let tools = sandbox.tools();
        let (release, held) = std::sync::mpsc::channel();
        let (entered, ready) = tokio::sync::oneshot::channel();
        // Occupy the actual bounded blocking pool so the real search is queued.
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            held.recv().unwrap();
        });
        ready.await.unwrap();
        let token = CancellationToken::new();
        let mut query = Box::pin(tools.execute_with_context(
            "grep",
            json!({"pattern":"needle"}),
            ToolExecutionContext {
                result_cap: 4096,
                cancellation: token.clone(),
            },
        ));
        let dispatched = matches!(
            futures_util::poll!(query.as_mut()),
            std::task::Poll::Pending
        );
        drop(query);
        let reason = token.reason();
        let mut settle = Box::pin(tools.settle_invocation());
        let retained = matches!(
            futures_util::poll!(settle.as_mut()),
            std::task::Poll::Pending
        );
        // Always release the pool before assertions, including baseline failures.
        release.send(()).unwrap();
        blocker.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(4), settle)
            .await
            .unwrap()
            .unwrap();
        assert!(
            dispatched,
            "actual search work must leave the async caller responsive"
        );
        assert_eq!(reason, Some(CancellationReason::OwnerFailure));
        assert!(
            retained,
            "abandoning the caller cannot settle a queued real worker"
        );
        assert_eq!(
            std::fs::read(sandbox.ws.root().join("entry.txt")).unwrap(),
            b"needle\n"
        );
        let next = tools
            .execute("grep", json!({"pattern":"needle"}))
            .await
            .unwrap();
        assert_eq!(
            next["total_matches"], 1,
            "drained owner permits the next real query"
        );
    });
}
