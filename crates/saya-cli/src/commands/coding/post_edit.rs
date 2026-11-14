//! Frozen post-edit diagnostics boundary; default-off is intentionally quiet.

use super::{CodingError, KnownWorkspaceMutation, checks::ChildCodingOperations};
use saya_agent::CancellationToken;
use saya_config::ExtensionProtocol;
use saya_harness::coding::child::OwnerFailureSnapshot;
use saya_harness::{
    coding::extensions::{ExtensionError, lsp::LspClient},
    workspace::Workspace,
};
use saya_types::CodingFailureClass;
use serde_json::json;

/// A fatal post-edit effect preserves known committed facts separately from
/// its payload-free owner failure classes.
#[derive(Debug, thiserror::Error)]
#[error("post-edit diagnostics could not be confirmed")]
pub(crate) struct PostEditFailure {
    committed: KnownWorkspaceMutation,
    owner_failure: OwnerFailureSnapshot,
}

impl PostEditFailure {
    pub(crate) fn new(
        committed: KnownWorkspaceMutation,
        owner_failure: OwnerFailureSnapshot,
    ) -> Self {
        Self {
            committed,
            owner_failure,
        }
    }

    pub(crate) fn committed(&self) -> &KnownWorkspaceMutation {
        &self.committed
    }

    pub(crate) fn owner_failure(&self) -> OwnerFailureSnapshot {
        self.owner_failure
    }
}

impl ChildCodingOperations {
    /// Whether this job's committed mutations are journaled: every in-place
    /// job's are (the folder is shared with the user), and a managed job's are
    /// when the automatic diagnostics profile is configured.
    pub(crate) fn journals_committed_mutations(&self) -> bool {
        self.application.is_in_place()
            || self
                .application
                .runtime
                .resolved
                .coding
                .automatic_lsp_profile
                .is_some()
    }

    /// Records one committed guarded mutation in the job's private journal.
    pub(crate) fn record_known_commit(
        &self,
        committed: &KnownWorkspaceMutation,
    ) -> Result<(), CodingError> {
        super::known_commit::append(self, committed)
    }

    /// Runs only after Luna's guarded publication owner supplies exact
    /// committed bytes. The trusted selector is necessary but never grants
    /// process authority; the immutable child runner scope is checked again.
    pub(crate) async fn post_edit_diagnostics(
        &self,
        committed: KnownWorkspaceMutation,
        cancellation: &CancellationToken,
    ) -> Result<Option<serde_json::Value>, PostEditFailure> {
        let Some(profile) = self
            .application
            .runtime
            .resolved
            .coding
            .automatic_lsp_profile
            .as_deref()
        else {
            return Ok(None);
        };
        let (config, profile, root) = self
            .extension(profile, ExtensionProtocol::Lsp, cancellation)
            .await
            .map_err(|_| {
                self.fatal(
                    committed.clone(),
                    CodingFailureClass::ExtensionExchange,
                    None,
                    cancellation,
                )
            })?;
        let workspace = Workspace::open(&root).map_err(|_| {
            self.fatal(
                committed.clone(),
                CodingFailureClass::ExtensionExchange,
                None,
                cancellation,
            )
        })?;
        let document = workspace
            .read(committed.path().as_str(), 128 * 1024)
            .map_err(|_| {
                self.fatal(
                    committed.clone(),
                    CodingFailureClass::ExtensionExchange,
                    None,
                    cancellation,
                )
            })?;
        if document.truncated {
            return Err(self.fatal(
                committed,
                CodingFailureClass::ExtensionExchange,
                None,
                cancellation,
            ));
        }
        if document.digest != committed.sha256().as_str() {
            return Ok(Some(stale(&committed, "before_exchange")));
        }
        let text = std::str::from_utf8(&document.bytes).map_err(|_| {
            self.fatal(
                committed.clone(),
                CodingFailureClass::ExtensionExchange,
                None,
                cancellation,
            )
        })?;
        let mut client = match LspClient::connect(config, cancellation).await {
            Ok(client) => client,
            Err(error) => return Err(self.extension_failure(committed, error, None, cancellation)),
        };
        let exchange = client
            .diagnostics(
                &root.join(committed.path().as_str()),
                profile.language().ok_or_else(|| {
                    self.fatal(
                        committed.clone(),
                        CodingFailureClass::ExtensionExchange,
                        None,
                        cancellation,
                    )
                })?,
                1,
                text,
                cancellation,
            )
            .await;
        let (shutdown, settlement) = client.shutdown_parts().await;
        if let Err(error) = settlement {
            let primary = if exchange.is_err() || shutdown.is_err() {
                CodingFailureClass::ExtensionExchange
            } else {
                CodingFailureClass::MandatoryPersistence
            };
            let secondary = (primary == CodingFailureClass::ExtensionExchange)
                .then_some(CodingFailureClass::MandatoryPersistence);
            return Err(self.extension_failure(
                committed,
                error,
                Some((primary, secondary)),
                cancellation,
            ));
        }
        if exchange.is_err() || shutdown.is_err() {
            return Ok(Some(unavailable(&committed)));
        }
        let fresh = workspace.read(committed.path().as_str(), 0).map_err(|_| {
            self.fatal(
                committed.clone(),
                CodingFailureClass::ExtensionExchange,
                None,
                cancellation,
            )
        })?;
        if fresh.digest != committed.sha256().as_str() {
            return Ok(Some(stale(&committed, "after_exchange")));
        }
        let diagnostics = exchange.expect("checked above");
        Ok(Some(json!({
            "status":"complete",
            "path":committed.path().as_str(),
            "document_sha256":committed.sha256(),
            "version":diagnostics.version,
            "diagnostics":diagnostics.diagnostics.into_iter().map(|item| json!({
                "line":item.line,
                "character":item.character,
                "severity":item.severity,
                "message":saya_types::redact(&item.message),
            })).collect::<Vec<_>>(),
            "edits_applied":false,
        })))
    }

    fn extension_failure(
        &self,
        committed: KnownWorkspaceMutation,
        error: ExtensionError,
        known: Option<(CodingFailureClass, Option<CodingFailureClass>)>,
        cancellation: &CancellationToken,
    ) -> PostEditFailure {
        let (primary, secondary) = known.unwrap_or(match error {
            ExtensionError::Observer => (CodingFailureClass::MandatoryPersistence, None),
            ExtensionError::ExchangeAndObserver => (
                CodingFailureClass::ExtensionExchange,
                Some(CodingFailureClass::MandatoryPersistence),
            ),
            ExtensionError::Configuration
            | ExtensionError::Unsupported
            | ExtensionError::Process
            | ExtensionError::Protocol
            | ExtensionError::Limit
            | ExtensionError::Timeout
            | ExtensionError::Cancelled
            | ExtensionError::ApprovalRequired
            | ExtensionError::UntrustedTool => (CodingFailureClass::ExtensionExchange, None),
        });
        self.fatal(committed, primary, secondary, cancellation)
    }

    fn fatal(
        &self,
        committed: KnownWorkspaceMutation,
        primary: CodingFailureClass,
        secondary: Option<CodingFailureClass>,
        cancellation: &CancellationToken,
    ) -> PostEditFailure {
        PostEditFailure::new(
            committed,
            self.fail_owner_operation(primary, secondary, cancellation),
        )
    }
}

fn unavailable(committed: &KnownWorkspaceMutation) -> serde_json::Value {
    json!({"status":"unavailable","path":committed.path().as_str(),"document_sha256":committed.sha256(),"version":1,"edits_applied":false})
}

fn stale(committed: &KnownWorkspaceMutation, phase: &str) -> serde_json::Value {
    json!({"status":"stale","path":committed.path().as_str(),"document_sha256":committed.sha256(),"version":1,"phase":phase,"edits_applied":false})
}
