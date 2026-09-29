//! The `investigation_run` tool (task Db): saved-investigation replay over
//! MCP through the same typed operation `saya investigation run` uses,
//! including the run command's `--param` bindings (Dc-1: the client's
//! `params` map reads as the same `name=value` strings, parsed by the same
//! typed parser; a refusal — a missing required parameter among them — is an
//! isError carrying the CLI's own words, and no error echoes a value). The
//! replay runs only against the startup allowlist (F-1, D1): inside the
//! serialized replay section — after the one-at-a-time slot is held — the
//! effective target is resolved exactly once (the `profile` argument, else
//! the saved binding's profile; no resolvable target refuses) and authorized
//! against the allowlist, and the command carries that target explicitly.
//! The run then re-reads the binding only to check staleness against the
//! authorized target, so a binding the ordinary CLI remaps underneath a
//! queued call can never widen the set this server serves (A922-1). The
//! review is never revalidated from here (invariant 2): a stale review is an
//! isError carrying the CLI's own message. The run path renders through the
//! process-output seam, so the call is wrapped in the capture the TUI replay
//! adapter uses — stdout stays protocol-only and the typed outcome, not the
//! rendered text, is what the client sees.

use rmcp::model::{CallToolRequestParams, CallToolResponse};
use saya_store::{InvestigationRepository, StoreError};
use saya_types::InvestigationId;
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
    // One capture window at a time (the capture is thread-local); the run
    // renders to the capture, never to the process stdout. The target is
    // resolved INSIDE the serialized section, after the slot is held (D1):
    // while a queued call waits here, the ordinary CLI can remap its
    // binding, so the binding must be read after the wait — anything read
    // before it can authorize a target that no longer is the one the run
    // would take.
    let _slot = context.replay_slot.lock().await;
    let target = match resolve_target(policy, context, id, profile) {
        Ok(target) => target,
        Err(message) => return Ok(tools::error_result(message)),
    };
    // The resolved target rides the command explicitly: the run re-reads the
    // binding only to check staleness against this target (never
    // `--revalidate` from here), so a binding remapped underneath a queued
    // call is refused there too — the two reads can never disagree silently.
    let command = crate::cli::InvestigationCommand::Run {
        id: id.to_owned(),
        connection: Some(target),
        revalidate: false,
        report: None,
        rows: None,
        overwrite: false,
        params,
    };
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

/// The replay's one concrete target (F-1, D1): resolved once, inside the
/// serialized replay section — the `profile` argument, else the saved
/// binding's profile — and authorized against the startup allowlist. The
/// caller passes the returned target to the run command explicitly, so the
/// run re-reads the binding only to check staleness against it; a binding
/// remapped underneath a queued call is refused there, never executed.
///
/// There is no pass-through: with neither an argument nor a readable
/// binding (nor a parseable id to read one for), the replay refuses
/// `profile not available` — a target the MCP layer could not name is not
/// something the run may resolve on its own at execution time (A922-1). A
/// client-supplied name is refused exactly as `query` refuses it, with the
/// name it asked for; a binding's profile name is never echoed — the
/// allowlist is also the client's information boundary — and a binding that
/// cannot be read fails the resolution closed.
pub(super) fn resolve_target(
    policy: &ServePolicy,
    context: &McpContext,
    id: &str,
    profile: Option<&str>,
) -> Result<String, String> {
    let Some(name) = profile else {
        let parsed = InvestigationId::parse(id).map_err(|_| "profile not available".to_owned())?;
        let bound =
            binding_target(context, &parsed).map_err(|_| "profile not available".to_owned())?;
        let bound = bound.ok_or_else(|| "profile not available".to_owned())?;
        if !policy
            .allowlist()
            .iter()
            .any(|summary| summary.name == bound)
        {
            return Err("profile not available".to_owned());
        }
        return Ok(bound);
    };
    context
        .allowed_profile(policy.allowlist(), name)
        .map(|_| name.to_owned())
}

/// The saved binding's profile: `Ok(Some)` when a binding reads cleanly,
/// `Ok(None)` when no binding exists — the resolution refuses — and `Err`
/// when the binding cannot be read, which the resolution fails closed on.
fn binding_target(
    context: &McpContext,
    parsed: &InvestigationId,
) -> Result<Option<String>, StoreError> {
    let repo = InvestigationRepository::new(context.runtime.investigations_root.clone());
    match repo.get_binding(parsed) {
        Ok(Some(binding)) => Ok(Some(binding.profile)),
        Ok(None) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Takes the captured output back if the future holding it is dropped — a
/// cancelled call must not leave a capture open for the next one to trip on.
struct CaptureGuard;

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        let _ = capture_output_take();
    }
}
