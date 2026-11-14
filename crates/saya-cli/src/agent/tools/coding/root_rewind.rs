//! The root's model-facing `rewind`: the front of the job's own checkpoint
//! store, advertised for the whole run.
//!
//! The tool is offered from the run's first request, before the first
//! checkpoint exists: a prompt cache is a prefix cache, so a tool that joined
//! the list mid-run would turn every later request into a cache miss. With no
//! checkpoint yet it answers truthfully — an empty listing, or the operation's
//! own refusal. It restores file content only, through the same store and
//! restore code the operator rewind uses, and it never runs a command, reaches
//! the network or changes conversation history.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
};
use serde_json::{Value, json};

use crate::commands::coding::{CodingError, NativeCodingJob};

/// The one model-tool name this module owns.
pub(crate) const REWIND: &str = "rewind";

/// The root's `rewind` tool over one job's checkpoint store.
pub(crate) struct RootRewind {
    job: Arc<NativeCodingJob>,
}

impl RootRewind {
    pub(crate) fn new(job: Arc<NativeCodingJob>) -> Self {
        Self { job }
    }

    pub(crate) fn definition() -> ToolDefinition {
        ToolDefinition {
            name: REWIND.into(),
            description: "Restore files in this run's workspace to a checkpoint the run \
                recorded. With no `checkpoint`, list the available checkpoints (id, turn, \
                changed paths). `checkpoint` is an id from that list or `last`; `paths` \
                restricts the restore to those workspace-relative files. Files only: it never \
                undoes a command, download or database effect. Prefer `edit` for small fixes."
                .into(),
            read_only: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "checkpoint": {
                        "type": "string",
                        "description": "An id from this tool's listing, or `last`."
                    },
                    "paths": {
                        "type": "array",
                        "maxItems": 512,
                        "items": {"type": "string", "maxLength": 512},
                        "description": "Restrict the restore to these workspace-relative files."
                    }
                },
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: true,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: ToolConcurrency::Serial,
            completion: Some("workspace files rewound".into()),
        }
    }
}

#[async_trait]
impl ToolExecutor for RootRewind {
    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        if name != REWIND {
            return Err(ToolError::UnsupportedTool);
        }
        let checkpoint = match arguments.get("checkpoint") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => Some(id.as_str()),
            Some(_) => {
                return Err(refused(
                    "`checkpoint` must be a string: an id from the listing, or `last`",
                ));
            }
        };
        let paths = match arguments.get("paths") {
            None | Some(Value::Null) => None,
            Some(Value::Array(values)) => {
                let mut paths = Vec::with_capacity(values.len());
                for value in values {
                    let Some(path) = value.as_str() else {
                        return Err(refused("`paths` must be an array of strings"));
                    };
                    paths.push(path.to_owned());
                }
                Some(paths)
            }
            Some(_) => return Err(refused("`paths` must be an array of strings")),
        };
        self.job
            .model_rewind(checkpoint, paths.as_deref())
            .await
            .map_err(map_error)
    }
}

fn refused(message: &str) -> ToolError {
    ToolError::NoEffect(Box::new(ToolError::Runner(message.into())))
}

/// Maps the operation's errors to the model's own words. A refusal raised
/// before the first write proves the call had no effect; a restore that fails
/// part-way is uncertain and stays a runner failure.
fn map_error(error: CodingError) -> ToolError {
    match error {
        CodingError::RewindUnknownTurn { .. } => {
            refused("no checkpoint has that id; call `rewind` with no `checkpoint` to list the ids")
        }
        CodingError::RewindNoCheckpoint => refused("this run has no checkpoint to rewind to"),
        CodingError::RewindNoCheckout => refused("this run has no working checkout to rewind"),
        CodingError::InvalidRequest => refused(
            "`checkpoint` must be an id from the listing or `last`, and `paths` must be \
             distinct workspace-relative files",
        ),
        other => ToolError::Runner(other.to_string()),
    }
}
