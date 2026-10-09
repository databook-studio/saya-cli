//! The refusal→model-text seam for checkpoint capture: whether a refused
//! commit provably left the file byte-identical (`Clean` — drop the
//! prepared record) or the effect may have landed (`Ambiguous` — keep it
//! as `Uncertain`), and the model-safe texts for screening and quota
//! refusals. Pure so every write-shaped tool classifies identically and
//! the mapping is unit-testable without a filesystem.

use saya_agent::ToolError;
use saya_store::ScreenRefusal;
use saya_types::CheckpointRefusal;

use super::CAPTURE_FAILED;

// The write tool's constant precondition refusals: the only provably clean
// `WorkspaceWrite` outcomes. Re-exported so the sibling tests classify
// against the exact texts rather than copies that can go stale.
pub(in crate::agent::tools) use crate::agent::tools::database_tools::{
    FILE_CHANGED_SINCE_OBSERVED, FILE_EXISTS_REQUIRES_DIGEST, MISSING_FILE_NOT_A_REPLACE,
};

/// Whether a refused edit provably changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::agent::tools) enum RefusalClass {
    /// A precondition mismatch raised before any write: the file is
    /// byte-identical and the prepared record is dropped.
    Clean,
    /// The effect may have landed (a harness failure, an identity change,
    /// I/O): the record is kept as `Uncertain`.
    Ambiguous,
}

/// Classifies a failed `workspace_edit` commit: the typed moved-anchor and
/// offset refusals are raised before any write; everything else from the
/// harness may have landed.
pub(in crate::agent::tools) fn classify_edit(error: &ToolError) -> RefusalClass {
    match error {
        ToolError::WorkspaceEditMoved { .. } | ToolError::WorkspaceAppendOffset { .. } => {
            RefusalClass::Clean
        }
        _ => RefusalClass::Ambiguous,
    }
}

/// Classifies a failed `workspace_write` commit: only the tool's three
/// constant precondition refusals are provably clean; any other harness
/// failure may have landed.
pub(in crate::agent::tools) fn classify_write(error: &ToolError) -> RefusalClass {
    match error {
        ToolError::WorkspaceWrite(text) => {
            if text == FILE_EXISTS_REQUIRES_DIGEST
                || text == FILE_CHANGED_SINCE_OBSERVED
                || text == MISSING_FILE_NOT_A_REPLACE
            {
                RefusalClass::Clean
            } else {
                RefusalClass::Ambiguous
            }
        }
        _ => RefusalClass::Ambiguous,
    }
}

/// The screening class's model-safe text: the class only, never the path
/// or the content.
pub(in crate::agent::tools) fn screen_class_text(class: ScreenRefusal) -> &'static str {
    match class {
        ScreenRefusal::CredentialPath => "credential path",
        ScreenRefusal::SecretContent => "secret-shaped content",
        ScreenRefusal::DatabaseExport => "database export",
        _ => "screened content",
    }
}

/// The quota refusal's model-safe text: names the quota class, promises
/// the file was not changed, never evicts. An unrecognized refusal falls
/// back to the generic capture failure.
pub(in crate::agent::tools) fn quota_refusal_text(quota: CheckpointRefusal) -> String {
    let class = match quota {
        CheckpointRefusal::FileTooLarge => "file too large",
        CheckpointRefusal::TooManyCheckpoints => "too many checkpoints",
        CheckpointRefusal::SessionBytesExceeded => "session recovery storage is full",
        _ => return CAPTURE_FAILED.to_owned(),
    };
    format!(
        "recovery is enabled but this edit cannot be checkpointed ({class}); \
         the file was not changed"
    )
}
