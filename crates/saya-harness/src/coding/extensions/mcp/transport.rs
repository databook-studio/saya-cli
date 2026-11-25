use super::super::{
    ExtensionConfig, ExtensionError, exchange_with_cleanup, framing::Framing,
    process::ProtocolProcess,
};
use super::{
    McpPolicy,
    policy::{valid_name, validate_tools},
};
use saya_agent::CancellationToken;
use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "2025-06-18";
/// What this client calls itself in `initialize`. The version is the crate's
/// own, so a release never has to remember a second copy of it.
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The `initialize` parameters this client sends: the protocol version, its
/// capabilities, and the client's own name and version.
fn initialize_params() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": {"name": "saya", "version": CLIENT_VERSION}
    })
}

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
        let response = match process
            .request("initialize", initialize_params(), cancel)
            .await
        {
            Ok(response) => response,
            Err(error) => return Err(exchange_with_cleanup(error, process.shutdown().await)),
        };
        if response.get("protocolVersion").and_then(Value::as_str) != Some(PROTOCOL_VERSION) {
            return Err(exchange_with_cleanup(
                ExtensionError::Protocol,
                process.shutdown().await,
            ));
        }
        if let Err(error) = process
            .notify("notifications/initialized", json!({}), cancel)
            .await
        {
            return Err(exchange_with_cleanup(error, process.shutdown().await));
        }
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
    pub async fn shutdown_parts(
        &mut self,
    ) -> (
        Result<(), ExtensionError>,
        Result<super::super::ExtensionProcessReceipt, ExtensionError>,
    ) {
        (Ok(()), self.process.shutdown_receipt().await)
    }
    pub async fn shutdown_with_receipt(
        mut self,
    ) -> Result<super::super::ExtensionProcessReceipt, ExtensionError> {
        self.process.shutdown_receipt().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_info_reports_the_crate_version() {
        let params = initialize_params();
        assert_eq!(
            params["clientInfo"]["version"].as_str(),
            Some(env!("CARGO_PKG_VERSION")),
            "clientInfo.version must be this crate's version, not a hand-copied one"
        );
    }
}
