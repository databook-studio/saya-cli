use super::super::{ExtensionConfig, ExtensionError, framing::Framing, process::ProtocolProcess};
use super::{DocumentDiagnostics, diagnostics::decode};
use saya_agent::CancellationToken;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Diagnostic-only server. Workspace edits and server requests are refused;
/// no proposed edit is ever applied by this client.
pub struct LspClient {
    process: ProtocolProcess,
    root: PathBuf,
}
impl LspClient {
    pub async fn connect(
        config: ExtensionConfig,
        cancel: &CancellationToken,
    ) -> Result<Self, ExtensionError> {
        if cancel.is_cancelled() {
            return Err(ExtensionError::Cancelled);
        }
        let root = config
            .spawn
            .fs_roots()
            .first()
            .ok_or(ExtensionError::Configuration)?
            .clone();
        let uri =
            url::Url::from_directory_path(&root).map_err(|_| ExtensionError::Configuration)?;
        let mut process = ProtocolProcess::start(config, Framing::ContentLength)?;
        process.request("initialize", json!({"processId":null, "rootUri":uri.as_str(),
            "capabilities":{"workspace":{"applyEdit":false},"textDocument":{"publishDiagnostics":{"versionSupport":true}}}}), cancel).await?;
        process.notify("initialized", json!({}), cancel).await?;
        Ok(Self { process, root })
    }

    /// Open a caller-selected existing document under the pinned workspace.
    /// Bounds apply before any protocol write. Versions are retained verbatim.
    pub async fn diagnostics(
        &mut self,
        path: &Path,
        language: &str,
        version: i64,
        text: &str,
        cancel: &CancellationToken,
    ) -> Result<DocumentDiagnostics, ExtensionError> {
        let canonical = path
            .canonicalize()
            .map_err(|_| ExtensionError::Configuration)?;
        if canonical != path
            || !canonical.starts_with(&self.root)
            || !canonical.is_file()
            || text.len() > 128 * 1024
            || language.is_empty()
            || language.len() > 64
            || language.chars().any(char::is_control)
            || !(0..=i64::from(i32::MAX)).contains(&version)
        {
            return Err(ExtensionError::Configuration);
        }
        let uri = url::Url::from_file_path(canonical)
            .map_err(|_| ExtensionError::Configuration)?
            .to_string();
        self.process
            .notify(
                "textDocument/didOpen",
                json!({"textDocument":{
            "uri":uri,"languageId":language,"version":version,"text":text}}),
                cancel,
            )
            .await?;
        let result = tokio::select! {
            _ = cancel.cancelled() => Err(ExtensionError::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(10), self.wait_diagnostics(&uri, version)) => result.unwrap_or(Err(ExtensionError::Timeout)),
        };
        if result.is_err() {
            // Cleanup owns the process receipt; it must not replace the
            // diagnostic operation's typed cause when graceful shutdown fails.
            let _ = self.process.shutdown().await;
        }
        result
    }

    async fn wait_diagnostics(
        &mut self,
        uri: &str,
        version: i64,
    ) -> Result<DocumentDiagnostics, ExtensionError> {
        for _ in 0..32 {
            let value = if self.process.notifications.is_empty() {
                self.process.receive().await?
            } else {
                self.process.notifications.remove(0)
            };
            if value.get("id").is_some() {
                return Err(ExtensionError::Protocol);
            }
            if value.get("method").and_then(Value::as_str)
                == Some("textDocument/publishDiagnostics")
            {
                let params = &value["params"];
                if params.get("uri").and_then(Value::as_str) != Some(uri) {
                    continue;
                }
                // A stale response is never published as current diagnostics.
                if params.get("version").and_then(Value::as_i64) != Some(version) {
                    continue;
                }
                return decode(params, uri, version);
            }
        }
        Err(ExtensionError::Limit)
    }
    pub async fn shutdown(self) -> Result<(), ExtensionError> {
        self.shutdown_with_receipt().await.map(|_| ())
    }
    pub async fn shutdown_with_receipt(
        mut self,
    ) -> Result<super::super::ExtensionProcessReceipt, ExtensionError> {
        let cancel = CancellationToken::new();
        let result = self.process.request("shutdown", Value::Null, &cancel).await;
        if result.is_ok() {
            let _ = self.process.notify("exit", Value::Null, &cancel).await;
        }
        let cleanup = self.process.shutdown_receipt().await;
        result.and(cleanup)
    }
}
