use super::support;
use saya_agent::CancellationToken;
use saya_harness::coding::extensions::{ExtensionError, mcp::McpClient};

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
