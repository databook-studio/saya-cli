use super::{CodingError, checks::ChildCodingOperations};
use saya_agent::CancellationToken;
use saya_config::{ExtensionProtocol, ExtensionToolEffect};
use saya_harness::{
    coding::{
        extensions::{
            ExtensionConfig, ExtensionError,
            lsp::LspClient,
            mcp::{McpClient, McpPolicy, McpToolEffect},
        },
        worktree::verify_owned_checkout,
    },
    runner::sandbox::{RunSandbox, SandboxProvision},
    workspace::Workspace,
};
use serde_json::{Value, json};
use std::path::PathBuf;
impl ChildCodingOperations {
    async fn extension(
        &self,
        profile: &str,
        protocol: ExtensionProtocol,
        token: &CancellationToken,
    ) -> Result<(ExtensionConfig, saya_config::ExtensionProfile, PathBuf), CodingError> {
        let profile = self
            .application
            .runtime
            .resolved
            .coding
            .extensions
            .get(profile)
            .filter(|profile| profile.protocol() == protocol)
            .ok_or(CodingError::InvalidRequest)?
            .clone();
        self.authority
            .admit_runner_program(profile.program())
            .map_err(CodingError::owner)?;
        let verified = verify_owned_checkout(&self.ownership(), token)
            .await
            .map_err(CodingError::owner)?;
        let root = PathBuf::from(verified.path().as_str());
        let sandbox =
            RunSandbox::new([root.clone()], std::iter::empty()).map_err(CodingError::owner)?;
        // The startup probe measures this host with real sandboxed children;
        // that measurement is synchronous and slow, so it runs on the
        // blocking pool and never stalls the job's runtime.
        let program_dir = self.application.program_dir.clone();
        let spawn = match tokio::task::spawn_blocking(move || sandbox.prepare(&program_dir))
            .await
            .map_err(CodingError::owner)?
            .map_err(CodingError::owner)?
        {
            SandboxProvision::Proven { spawn, .. } => spawn,
            _ => return Err(CodingError::ConfinementUnavailable),
        };
        let config = ExtensionConfig::new(
            spawn,
            self.application.program_dir.join(profile.program()),
            profile.argv().to_vec(),
        )
        // Checked before any server process exists; the usual cause is a
        // symlinked program (a distribution's `sh` or `python3`).
        .map_err(|_| CodingError::ExtensionProgramRefused)?;
        let config = config.with_observer(std::sync::Arc::new(
            super::extension_observer::ProcessObserver::new(self),
        ));
        // No inherited credentials and no implicit secret injection. A program
        // grant is not a secret grant; profile data never widens the root.
        Ok((config, profile, root))
    }
    pub(crate) async fn mcp(
        &self,
        profile: &str,
        tool: Option<&str>,
        arguments: &Value,
        token: &CancellationToken,
    ) -> Result<Value, CodingError> {
        let (config, profile, _) = self
            .extension(profile, ExtensionProtocol::Mcp, token)
            .await?;
        let policy = McpPolicy::new(
            profile
                .tools()
                .iter()
                .map(|(name, effect)| {
                    (
                        name.clone(),
                        match effect {
                            ExtensionToolEffect::ReadOnly => McpToolEffect::ReadOnly,
                            ExtensionToolEffect::Mutating => McpToolEffect::Mutating,
                            ExtensionToolEffect::Unknown => McpToolEffect::Unknown,
                        },
                    )
                })
                .collect(),
        )
        .map_err(CodingError::owner)?;
        if let Some(tool) = tool
            && let Err(error) = policy.authorize(tool)
        {
            // No server was contacted and no process spawned: the refusal is
            // the tool's whole result, and no effect permit stays uncertain.
            return Ok(denial_outcome(&error));
        }
        let mut client = McpClient::connect(config, token)
            .await
            .map_err(CodingError::owner)?;
        let result = async {
            let listing = client.list_tools(token).await?;
            match tool {
                Some(tool) => client.call_tool(tool, arguments, &policy, token).await,
                None => Ok(listing),
            }
        }
        .await;
        let shutdown = client.shutdown().await;
        shutdown.map_err(CodingError::owner)?;
        match result {
            Ok(value) => Ok(value),
            // A bounded exchange ends deterministically: the policy refused,
            // or the client's bound rejected a listing or response. The
            // spawned process still settled through the receipt, so the
            // effect is known — the denial is data for the model, the way a
            // check's receipt carries a known nonzero exit.
            Err(error) if bounded_denial(&error) => Ok(denial_outcome(&error)),
            Err(error) => Err(CodingError::owner(error)),
        }
    }
    pub(crate) async fn lsp(
        &self,
        profile: &str,
        path: &str,
        version: i64,
        token: &CancellationToken,
    ) -> Result<Value, CodingError> {
        let (config, profile, root) = self
            .extension(profile, ExtensionProtocol::Lsp, token)
            .await?;
        let workspace = Workspace::open(&root).map_err(CodingError::owner)?;
        let document = workspace
            .read(path, 128 * 1024)
            .map_err(CodingError::owner)?;
        if document.truncated {
            return Err(CodingError::InvalidRequest);
        }
        let text = std::str::from_utf8(&document.bytes).map_err(CodingError::owner)?;
        let mut client = LspClient::connect(config, token)
            .await
            .map_err(CodingError::owner)?;
        let result = client
            .diagnostics(
                &root.join(path),
                profile.language().ok_or(CodingError::InvalidRequest)?,
                version,
                text,
                token,
            )
            .await;
        let shutdown = client.shutdown().await;
        shutdown.map_err(CodingError::owner)?;
        let diagnostics = result.map_err(CodingError::owner)?;
        let fresh = workspace.read(path, 0).map_err(CodingError::owner)?;
        if fresh.digest != document.digest {
            return Err(CodingError::StaleReview);
        }
        Ok(
            json!({"path":path,"document_sha256":document.digest,"version":diagnostics.version,"diagnostics":diagnostics.diagnostics.iter().map(|item|json!({"line":item.line,"character":item.character,"severity":item.severity,"message":saya_types::redact(&item.message)})).collect::<Vec<_>>(),"edits_applied":false}),
        )
    }
}

/// Whether one MCP exchange error is a bounded, deterministic denial rather
/// than a job-failing uncertainty: the trusted policy refused the tool, the
/// client's own bound rejected a listing or response, or the server broke the
/// exchange (pipe or timeout) while the spawned process still settles through
/// its receipt. Cancellation, configuration, and platform gaps stay errors.
fn bounded_denial(error: &ExtensionError) -> bool {
    matches!(
        error,
        ExtensionError::UntrustedTool
            | ExtensionError::ApprovalRequired
            | ExtensionError::Protocol
            | ExtensionError::Limit
            | ExtensionError::Process
            | ExtensionError::Timeout
    )
}

/// The denial as the tool's data result: what the bounded exchange decided
/// and the fixed reason text. No server content rides in a denial.
fn denial_outcome(error: &ExtensionError) -> Value {
    let outcome = match error {
        ExtensionError::UntrustedTool | ExtensionError::ApprovalRequired => "refused",
        ExtensionError::Limit => "dropped",
        ExtensionError::Process | ExtensionError::Timeout => "failed",
        _ => "rejected",
    };
    json!({"outcome":outcome,"reason":error.to_string()})
}
