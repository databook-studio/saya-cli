//! The `investigation_run` tool (task Db): saved-investigation replay over
//! MCP through the same typed operation `saya investigation run` uses,
//! including the run command's `--param` bindings (Dc-1: the client's
//! `params` map reads as the same `name=value` strings, parsed by the same
//! typed parser; a refusal — a missing required parameter among them — is an
//! isError carrying the CLI's own words, and no error echoes a value). The
//! review is never revalidated from here (invariant 2): a stale review is an
//! isError carrying the CLI's own message. The run path renders through the
//! process-output seam, so the call is wrapped in the capture the TUI replay
//! adapter uses — stdout stays protocol-only and the typed outcome, not the
//! rendered text, is what the client sees.

use rmcp::model::{CallToolRequestParams, CallToolResponse};
use serde_json::json;

use super::{context::McpContext, policy::ServePolicy, tools};
use crate::commands::{
    Replay, capture_output_start, capture_output_take, run_investigation_outcome,
};
use crate::render::RenderFormat;

pub(crate) async fn investigation_run(
    policy: &ServePolicy,
    context: &McpContext,
    request: &CallToolRequestParams,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    let id = tools::required_string(request, "id")?;
    let profile = tools::optional_string(request, "profile")?;
    let params = tools::optional_string_map(request, "params")?;
    let command = crate::cli::InvestigationCommand::Run {
        id: id.to_owned(),
        connection: profile.map(str::to_owned),
        revalidate: false,
        report: None,
        rows: None,
        overwrite: false,
        params,
    };
    // One capture window at a time (the capture is thread-local); the run
    // renders to the capture, never to the process stdout.
    let _slot = context.replay_slot.lock().await;
    capture_output_start();
    let guard = CaptureGuard;
    let outcome = run_investigation_outcome(
        command,
        &context.runtime,
        RenderFormat::Text,
        false,
        &context.store,
    )
    .await;
    let (out, err) = capture_output_take();
    std::mem::forget(guard);
    match outcome {
        Ok(outcome) if outcome.code == 0 => match outcome.replay {
            Some(replay) => tools::bounded_result(policy, replay_payload(replay)),
            None => Ok(tools::error_result("the replay did not produce a result")),
        },
        Ok(_) => {
            // A refusal or a failed run: the CLI's own words (the captured
            // message), never a revalidation.
            let rendered = if out.trim().is_empty() { err } else { out };
            Ok(tools::error_result(rendered))
        }
        Err(error) => Ok(tools::error_result(error.to_string())),
    }
}

fn replay_payload(replay: Replay) -> serde_json::Value {
    json!({
        "result": replay.result,
        "evidence": replay.evidence,
        "connection": replay.connection,
    })
}

/// Takes the captured output back if the future holding it is dropped — a
/// cancelled call must not leave a capture open for the next one to trip on.
struct CaptureGuard;

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        let _ = capture_output_take();
    }
}
