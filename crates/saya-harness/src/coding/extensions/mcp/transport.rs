use super::super::{ExtensionConfig, ExtensionError, framing::Framing, process::ProtocolProcess};
use super::{
    McpPolicy,
    policy::{valid_name, validate_tools},
};
use saya_agent::CancellationToken;
use serde_json::{Value, json};

/// One serialized, bounded MCP stdio connection. No external agent fallback.
pub struct McpClient {
    process: ProtocolProcess,
    tools: Vec<String>,
}
impl McpClient {
    pub async fn connect(
        config: ExtensionConfig,
        cancel: &CancellationToken,
    ) -> Result<Self, ExtensionError> {
        if cancel.is_cancelled() {
            return Err(ExtensionError::Cancelled);
        }
        let mut process = ProtocolProcess::start(config, Framing::Lines)?;
        let response = process
            .request(
                "initialize",
                json!({
                    "protocolVersion":"2024-11-05", "capabilities":{},
                    "clientInfo":{"name":"saya", "version":"0.5.0"}
                }),
                cancel,
            )
            .await?;
        if response.get("protocolVersion").and_then(Value::as_str) != Some("2024-11-05") {
            process.shutdown().await?;
            return Err(ExtensionError::Protocol);
        }
        process
            .notify("notifications/initialized", json!({}), cancel)
            .await?;
        Ok(Self {
            process,
            tools: Vec::new(),
        })
    }
    pub async fn list_tools(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Value, ExtensionError> {
        let result = self
            .process
            .request("tools/list", json!({}), cancel)
            .await?;
        validate_tools(&result)?;
        self.tools = result["tools"]
            .as_array()
            .ok_or(ExtensionError::Protocol)?
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect();
        Ok(result)
    }
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: &Value,
        policy: &McpPolicy,
        cancel: &CancellationToken,
    ) -> Result<Value, ExtensionError> {
        policy.authorize(name)?;
        if !valid_name(name)
            || !self.tools.iter().any(|tool| tool == name)
            || !arguments.is_object()
        {
            return Err(ExtensionError::Protocol);
        }
        self.process
            .request(
                "tools/call",
                json!({"name":name,"arguments":arguments}),
                cancel,
            )
            .await
    }
    pub async fn shutdown(self) -> Result<(), ExtensionError> {
        self.shutdown_with_receipt().await.map(|_| ())
    }
    pub async fn shutdown_with_receipt(
        mut self,
    ) -> Result<super::super::ExtensionProcessReceipt, ExtensionError> {
        self.process.shutdown_receipt().await
    }
}
