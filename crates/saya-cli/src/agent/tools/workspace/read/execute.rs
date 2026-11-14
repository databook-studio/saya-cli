use std::sync::Arc;

use saya_agent::{ToolError, ToolExecutionContext, shape_tool_result};
use saya_harness::{
    HarnessError,
    workspace::{ControlledReadError, MAX_IO_BYTES},
};
use serde_json::Value;

use super::super::super::database_tools::DatabaseTools;
use super::super::notebook;
use super::super::search_lifetime::{CancellationOnDrop, validate_context};
use super::super::witness::{ReadTicket, ReadWitnesses};
use super::{MAX_IMAGE_BYTES, WORKSPACE_READ_MAX_BYTES, WORKSPACE_READ_MAX_LINES};

/// The one line a read adds when the content it serves is exactly what this
/// run's last write or edit of the path produced.
const UNCHANGED_SINCE_LAST_EDIT: &str = "unchanged since your last edit";

pub(super) mod image;
mod projection;
use self::projection::{ranged_result, unranged_result};

struct ReadObservationGuard {
    witnesses: Arc<ReadWitnesses>,
    ticket: ReadTicket,
}

impl Drop for ReadObservationGuard {
    fn drop(&mut self) {
        self.witnesses.finish_read_worker(&self.ticket);
    }
}

pub(super) async fn execute(
    tools: &DatabaseTools,
    arguments: &Value,
    context: ToolExecutionContext,
) -> Result<Value, ToolError> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or(ToolError::PathNotString)?
        .to_owned();
    let Some(workspace) = tools.workspace.as_ref().cloned() else {
        return Err(ToolError::WorkspaceUnavailable);
    };
    validate_context(&context, "workspace read")?;
    let range = requested_range(arguments)?;
    ensure_metadata_fits(&path, context.result_cap, range)?;
    // This job's own shell-output spool is the one place outside the
    // workspace the read tool reaches. It is opened as its own workspace, so
    // the same containment and bounds apply; no witness is recorded for it.
    if let Some(target) = tools.spool_target(&path) {
        let retained = if range.is_some() {
            MAX_IO_BYTES as u64
        } else {
            WORKSPACE_READ_MAX_BYTES
        };
        let file =
            target
                .workspace
                .read(&target.relative, retained)
                .map_err(|error| match error.is_not_found() {
                    true => ToolError::WorkspaceFileMissing,
                    false => ToolError::Workspace(error.to_string()),
                })?;
        let (result, _, _) = match range {
            Some(range) => ranged_result(&path, file, range, context.result_cap)?,
            None => unranged_result(&path, file, context.result_cap)?,
        };
        return Ok(result);
    }
    let worker_guard = tools.workspace_tasks.begin()?;
    let ticket = tools.read_witnesses.begin_read(&path);
    let observe_guard = ticket.as_ref().map(|ticket| ReadObservationGuard {
        witnesses: Arc::clone(&tools.read_witnesses),
        ticket: ticket.clone(),
    });
    let cancellation = context.cancellation.clone();
    let mut cancel_on_drop = CancellationOnDrop::new(cancellation.clone());
    let worker_workspace = Arc::clone(&workspace);
    let worker_path = path.clone();
    // An image candidate is read up to its own bound — below the ranged read's
    // retention, so the memory bound this path can hold is never raised. A
    // notebook is read whole so its cells can be parsed at all: an embedded
    // image puts it far past the text read's bound, and its own bound is the
    // ranged read's, never more.
    let retained = if image::is_candidate(&path) {
        MAX_IMAGE_BYTES
    } else if range.is_some() {
        MAX_IO_BYTES as u64
    } else if notebook::is_candidate(&path) {
        notebook::NOTEBOOK_MAX_BYTES
    } else {
        WORKSPACE_READ_MAX_BYTES
    };
    let worker = tokio::task::spawn_blocking(move || {
        let _observe_guard = observe_guard;
        (
            worker_workspace.read_controlled(&worker_path, retained, &cancellation),
            worker_guard,
        )
    });
    let (read, worker_guard) = worker
        .await
        .map_err(|_| ToolError::Runner("workspace read worker outcome is unavailable".into()))?;
    cancel_on_drop.disarm();
    let file = match read {
        Ok(file) => file,
        Err(ControlledReadError::Workspace(HarnessError::NotRegularFile { .. }))
            if range.is_none() =>
        {
            drop(worker_guard);
            return tools.workspace_list(arguments, context).await;
        }
        Err(error) => return Err(map_read_error(error)),
    };
    if context.cancellation.is_cancelled() {
        return Err(no_effect("workspace read was cancelled"));
    }

    // A file whose name says image and whose bytes carry that image's
    // signature is served as an image; everything else keeps the text path.
    let detected = image::is_candidate(&path)
        .then(|| image::media_type(&file.bytes))
        .flatten();
    let unchanged_since_edit =
        tools
            .read_witnesses
            .last_mutation_matches(&path, &workspace, &file.digest);
    let (mut result, digest, size) = match detected {
        Some(media_type) => {
            if range.is_some() {
                return Err(ToolError::WorkspaceImageLineRange { path: path.clone() });
            }
            image::result(&path, media_type, file)?
        }
        None => match range {
            // A `.ipynb` that really is a notebook is served as its cells; one
            // whose bytes say otherwise — a text file with that name, a
            // notebook too large for the retained read — keeps the ordinary
            // text path. Either way the digest and size are the file's own.
            None => match notebook::is_candidate(&path)
                .then(|| notebook::scan(&file.bytes))
                .flatten()
            {
                Some(cells) => notebook::result(&path, file, &cells, context.result_cap)?,
                None => unranged_result(&path, file, context.result_cap)?,
            },
            Some(range) => ranged_result(&path, file, range, context.result_cap)?,
        },
    };
    if unchanged_since_edit {
        result["note"] = Value::String(UNCHANGED_SINCE_LAST_EDIT.into());
    }
    if context.cancellation.is_cancelled() {
        return Err(no_effect("workspace read was cancelled"));
    }
    if let Some(ticket) = ticket {
        tools
            .read_witnesses
            .observe_read(ticket, &workspace, digest.clone(), size);
    }
    drop(worker_guard);
    Ok(result)
}

#[derive(Clone, Copy)]
pub(super) struct LineRange {
    start: u64,
    count: u64,
}

fn requested_range(arguments: &Value) -> Result<Option<LineRange>, ToolError> {
    let Some(object) = arguments.as_object() else {
        return Err(ToolError::ArgumentsNotObject);
    };
    if !object.contains_key("start_line") && !object.contains_key("line_count") {
        return Ok(None);
    }
    let start = object
        .get("start_line")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let count = object
        .get("line_count")
        .and_then(Value::as_u64)
        .unwrap_or(WORKSPACE_READ_MAX_LINES);
    if start == 0
        || count == 0
        || count > WORKSPACE_READ_MAX_LINES
        || start.checked_add(count - 1).is_none()
    {
        return Err(ToolError::UnsupportedProperty);
    }
    Ok(Some(LineRange { start, count }))
}

fn ensure_metadata_fits(path: &str, cap: usize, range: Option<LineRange>) -> Result<(), ToolError> {
    let mut result = serde_json::json!({
        "path": path,
        "size": u64::MAX,
        "truncated": true,
        "digest": "0".repeat(64),
        "content": "",
    });
    if let Some(range) = range {
        result["start_line"] = range.start.into();
        result["line_count"] = range.count.into();
        result["total_lines"] = u64::MAX.into();
        result["lines_returned"] = range.count.into();
        result["selected_range_complete"] = true.into();
    }
    if shape_tool_result(&result, cap).truncated {
        return Err(no_effect("workspace result cap cannot hold read metadata"));
    }
    Ok(())
}

fn map_read_error(error: ControlledReadError) -> ToolError {
    match error {
        ControlledReadError::Cancelled => no_effect("workspace read was cancelled"),
        ControlledReadError::Workspace(error) if error.is_not_found() => {
            ToolError::WorkspaceFileMissing
        }
        ControlledReadError::Workspace(error) => ToolError::Workspace(error.to_string()),
    }
}

fn no_effect(message: &'static str) -> ToolError {
    ToolError::NoEffect(Box::new(ToolError::Runner(message.into())))
}
