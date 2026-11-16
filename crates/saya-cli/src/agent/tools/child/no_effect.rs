//! Which child write/edit refusals provably changed nothing: the typed
//! precondition refusals raised before any write, the malformed-argument
//! refusals, and the exact clean workspace constant refusals. Pure so the
//! executor wraps exactly these and nothing else.

use saya_agent::ToolError;
use saya_types::CheckpointRefusal;

use super::super::workspace::checkpoint::{RefusalClass, classify_write, quota_refusal_text};

/// Whether a failed child write or edit provably changed nothing. The typed
/// refusals here are all raised before any write lands. Only exact recognized
/// workspace refusal constants prove a stringly error was clean; everything
/// else — a harness failure, unrecognized refusal text, runner or fetch error —
/// may have landed and stays uncertain.
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
        | ToolError::WorkspaceDigestRequired { .. }
        | ToolError::WorkspaceWriteMoved { .. }
        // Every notebook refusal is raised before the commit: an unknown mode
        // or cell, a file that is not a notebook, and the result-is-not-JSON
        // self-check all precede any byte being written.
        | ToolError::NotebookModeUnsupported { .. }
        | ToolError::WorkspaceNotebook { .. }
        | ToolError::PathNotString
        | ToolError::ContentNotString
        | ToolError::ArgumentsNotObject
        | ToolError::UnsupportedProperty
        | ToolError::ExpectedSizeNotUint
        | ToolError::ExpectedDigestNotString
        | ToolError::ReplaceAllNotBool
        | ToolError::ReplaceAllConflict
        | ToolError::UnsupportedTool
        | ToolError::WorkspaceUnavailable => true,
        ToolError::NoEffect(_) => true,
        ToolError::WorkspaceWrite(_) => {
            classify_write(error) == RefusalClass::Clean || is_checkpoint_quota_refusal(error)
        }
        ToolError::WorkspaceEdit(text) => {
            is_checkpoint_quota_refusal(error)
                || text == super::super::workspace::MULTI_EDIT_OVERLAP_REFUSAL
        }
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
