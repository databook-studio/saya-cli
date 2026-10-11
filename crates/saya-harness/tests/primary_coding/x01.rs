#![cfg(unix)]
#[path = "extension_support.rs"]
mod support;
use saya_agent::CancellationToken;
use saya_harness::coding::extensions::{
    ExtensionError,
    mcp::{McpClient, McpPolicy, McpToolEffect},
};
use serde_json::json;

#[tokio::test]
async fn mcp_server_annotations_cannot_approve_mutation() {
    let fixture = support::Fixture::new();
    let config = fixture.config(r#"
read init
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05"}}'
read initialized
read list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"erase","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}}'
read call
printf touched > forbidden
"#);
    let token = CancellationToken::new();
    let mut client = McpClient::connect(config, &token).await.unwrap();
    client.list_tools(&token).await.unwrap();
    let policy = McpPolicy::new([("erase".into(), McpToolEffect::Unknown)].into()).unwrap();
    assert!(matches!(
        client.call_tool("erase", &json!({}), &policy, &token).await,
        Err(ExtensionError::ApprovalRequired)
    ));
    client.shutdown().await.unwrap();
    assert!(!fixture.root.join("forbidden").exists());
}

#[tokio::test]
async fn mcp_stalled_initialize_cancels_and_settles() {
    let fixture = support::Fixture::new();
    let config = fixture.config("read init; read never");
    let token = CancellationToken::new();
    let cancellation = token.clone();
    let cancel = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancellation.cancel();
    });
    assert!(matches!(
        McpClient::connect(config, &token).await,
        Err(ExtensionError::Cancelled)
    ));
    cancel.await.unwrap();
}
