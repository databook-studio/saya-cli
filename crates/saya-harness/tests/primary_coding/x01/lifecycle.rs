use super::support;
use saya_agent::CancellationToken;
use saya_harness::coding::extensions::{
    ExtensionError,
    mcp::{McpClient, McpPolicy, McpToolEffect},
};
use serde_json::json;

#[tokio::test]
async fn mcp_transport_refuses_dead_malformed_wrong_id_and_oversized_servers() {
    for script in [
        "exit 0",
        "read init; printf '%s\\n' 'not-json'",
        "read init; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{\"protocolVersion\":\"2025-06-18\"}}'",
        "read init; head -c 1048577 /dev/zero",
    ] {
        let fixture = support::Fixture::new();
        assert!(matches!(
            McpClient::connect(fixture.config(script), &CancellationToken::new()).await,
            Err(ExtensionError::Protocol | ExtensionError::Limit | ExtensionError::Process(_))
        ));
    }
}

#[tokio::test]
async fn mcp_normal_handshake_list_call_and_shutdown_use_public_transport_owner() {
    let fixture = support::Fixture::new();
    let config = fixture.config(r#"
read init
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}}'
read initialized
read list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"read","inputSchema":{"type":"object"}}]}}'
read call
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[]}}'
"#);
    let token = CancellationToken::new();
    let mut client = McpClient::connect(config, &token).await.unwrap();
    client.list_tools(&token).await.unwrap();
    client.shutdown_with_receipt().await.unwrap();
}

#[tokio::test]
async fn mcp_refuses_paged_schema_and_oversized_outbound_calls_at_public_boundary() {
    for tools in [
        r#"{"tools":[],"nextCursor":"more"}"#,
        r#"{"tools":[{"name":"read","inputSchema":{"type":"array"}}]}"#,
    ] {
        let fixture = support::Fixture::new();
        let config = fixture.config(&format!(
            r#"
read init
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2025-06-18"}}}}'
read initialized
read list
printf '%s\n' '{tools}'
"#
        ));
        let token = CancellationToken::new();
        let mut client = McpClient::connect(config, &token).await.unwrap();
        assert!(matches!(
            client.list_tools(&token).await,
            Err(ExtensionError::Limit | ExtensionError::Protocol)
        ));
    }
    let fixture = support::Fixture::new();
    let config = fixture.config(r#"
read init; printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}}'
read initialized; read list; printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"read","inputSchema":{"type":"object"}}]}}'
"#);
    let token = CancellationToken::new();
    let mut client = McpClient::connect(config, &token).await.unwrap();
    client.list_tools(&token).await.unwrap();
    let policy = McpPolicy::new([("read".into(), McpToolEffect::ReadOnly)].into()).unwrap();
    assert!(matches!(
        client
            .call_tool(
                "read",
                &json!({"x":"x".repeat(1024 * 1024)}),
                &policy,
                &token
            )
            .await,
        Err(ExtensionError::Limit)
    ));
}

#[tokio::test]
async fn mcp_live_call_cancellation_uses_public_process_owner() {
    let fixture = support::Fixture::new();
    let config = fixture.config(r#"
read init; printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}}'
read initialized; read list; printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"read","inputSchema":{"type":"object"}}]}}'
read call; sleep 30
"#);
    let token = CancellationToken::new();
    let mut client = McpClient::connect(config, &token).await.unwrap();
    client.list_tools(&token).await.unwrap();
    let policy = McpPolicy::new([("read".into(), McpToolEffect::ReadOnly)].into()).unwrap();
    let cancel = token.clone();
    let task =
        tokio::spawn(async move { client.call_tool("read", &json!({}), &policy, &cancel).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    token.cancel();
    assert!(matches!(
        task.await.unwrap(),
        Err(ExtensionError::Cancelled)
    ));
}
