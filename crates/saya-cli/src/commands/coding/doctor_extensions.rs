use super::CodingApplication;
use saya_agent::CancellationToken;
use saya_harness::{
    coding::extensions::{ExtensionConfig, lsp::LspClient, mcp::McpClient},
    runner::sandbox::RunnerSpawn,
};
use serde_json::{Value, json};
impl CodingApplication {
    /// Explicit doctor --probe: fixed trusted profiles run only in the fresh
    /// empty private probe checkout, with no credentials or network authority.
    pub(super) async fn probe_extensions(&self, spawn: &RunnerSpawn) -> Vec<Value> {
        let mut results = Vec::new();
        for (name, profile) in &self.runtime.resolved.coding.extensions {
            let token = CancellationToken::new();
            let config = ExtensionConfig::new(
                spawn.clone(),
                self.program_dir.join(profile.program()),
                profile.argv().to_vec(),
            );
            let healthy = match (config, profile.protocol()) {
                (Ok(config), saya_config::ExtensionProtocol::Mcp) => {
                    match McpClient::connect(config, &token).await {
                        Ok(mut client) => {
                            let listing = client.list_tools(&token).await;
                            let closed = client.shutdown().await;
                            listing.is_ok() && closed.is_ok()
                        }
                        Err(_) => false,
                    }
                }
                (Ok(config), saya_config::ExtensionProtocol::Lsp) => {
                    match LspClient::connect(config, &token).await {
                        Ok(client) => client.shutdown().await.is_ok(),
                        Err(_) => false,
                    }
                }
                _ => false,
            };
            results.push(json!({"profile":name,"healthy":healthy}));
        }
        results
    }
}
