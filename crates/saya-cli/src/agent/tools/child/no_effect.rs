//! Which child write/edit refusals provably changed nothing: the typed
//! precondition refusals raised before any write, the malformed-argument
//! refusals, and the write tool's clean constant refusals. Pure so the
//! executor wraps exactly these and nothing else.

use saya_agent::ToolError;
use saya_types::CheckpointRefusal;

use super::super::workspace::checkpoint::{RefusalClass, classify_write, quota_refusal_text};

/// Whether a failed child write or edit provably changed nothing. The typed
/// refusals here are all raised before any write lands; the write tool's
/// constant precondition refusals are the only clean `WorkspaceWrite`
/// outcomes. Everything else — a harness failure, an unrecognised refusal
/// text, a runner or fetch error — may have landed and stays uncertain.
pub(super) fn proves_no_effect(error: &ToolError) -> bool {
    match error {
        ToolError::WorkspaceEditNoMatch { .. }
        | ToolError::WorkspaceEditAmbiguous { .. }
        | ToolError::WorkspaceEditMoved { .. }
        | ToolError::WorkspaceEditEmptyAnchor { .. }
        | ToolError::WorkspaceEditTooLarge { .. }
        | ToolError::WorkspaceEditNotText { .. }
        | ToolError::WorkspaceAppendOffset { .. }
        | ToolError::WorkspaceWriteTooLarge { .. }
        | ToolError::PathNotString
        | ToolError::ContentNotString
        | ToolError::ArgumentsNotObject
        | ToolError::UnsupportedProperty
        | ToolError::ExpectedSizeNotUint
        | ToolError::ExpectedDigestNotString
        | ToolError::UnsupportedTool
        | ToolError::WorkspaceUnavailable => true,
        ToolError::WorkspaceWrite(_) => {
            classify_write(error) == RefusalClass::Clean || is_checkpoint_quota_refusal(error)
        }
        ToolError::WorkspaceEdit(_) => is_checkpoint_quota_refusal(error),
        _ => false,
    }
}

fn is_checkpoint_quota_refusal(error: &ToolError) -> bool {
    let text = match error {
        ToolError::WorkspaceWrite(text) | ToolError::WorkspaceEdit(text) => text,
        _ => return false,
    };
    [
        CheckpointRefusal::FileTooLarge,
        CheckpointRefusal::TooManyCheckpoints,
        CheckpointRefusal::SessionBytesExceeded,
    ]
    .into_iter()
    .any(|quota| quota_refusal_text(quota) == *text)
}
