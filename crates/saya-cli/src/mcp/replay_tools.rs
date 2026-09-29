//! The `investigation_run` tool (task Db): saved-investigation replay over
//! MCP through the same typed operation `saya investigation run` uses,
//! including the run command's `--param` bindings (Dc-1: the client's
//! `params` map reads as the same `name=value` strings, parsed by the same
//! typed parser; a refusal — a missing required parameter among them — is an
//! isError carrying the CLI's own words, and no error echoes a value). The
//! replay runs only against the startup allowlist (F-1): the effective
//! target — the `profile` argument, else the saved binding's profile — is
//! resolved exactly as the run command resolves it and must sit inside the
//! allowlist before anything runs. The review is never revalidated from here
//! (invariant 2): a stale review is an isError carrying the CLI's own
//! message. The run path renders through the process-output seam, so the
//! call is wrapped in the capture the TUI replay adapter uses — stdout stays
//! protocol-only and the typed outcome, not the rendered text, is what the
//! client sees.

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
    // The replay stays inside the startup allowlist (F-1): the effective
    // target is resolved exactly as the run command resolves it, and a
    // target outside the allowlist is refused before anything runs — never
    // handed to the run.
    if let Err(message) = allowlist_gate(policy, context, id, profile) {
        return Ok(tools::error_result(message));
    }
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

/// The replay's allowlist gate (F-1): the effective target — the `profile`
/// argument, else the saved binding's profile, the run command's own
/// resolution — must sit inside the startup allowlist before anything runs.
/// A binding saved against a profile this server does not serve is not a
/// licence to reach it. `Err` is the isError text. With neither an argument
/// nor a binding the gate passes: the run command refuses that with the
/// CLI's own words, in the CLI's own order, before any connection.
pub(super) fn allowlist_gate(
    policy: &ServePolicy,
    context: &McpContext,
    id: &str,
    profile: Option<&str>,
) -> Result<(), String> {
    match profile {
        // A client-supplied name is refused exactly as `query` refuses it.
        Some(name) => context
            .allowed_profile(policy.allowlist(), name)
            .map(|_| ()),
        None => {
            let Ok(parsed) = InvestigationId::parse(id) else {
                // A malformed id cannot have a binding; the run command
                // refuses it with its own words before anything runs.
                return Ok(());
            };
            // A binding that cannot be read cannot be checked, so the read
            // error is the refusal: the gate fails closed (F-1 follow-up),
            // and neither the store error's own text nor the bound profile
            // name is echoed.
            let bound =
                binding_target(context, &parsed).map_err(|_| "profile not available".to_owned())?;
            match bound {
                Some(target)
                    if !policy
                        .allowlist()
                        .iter()
                        .any(|summary| summary.name == target) =>
                {
                    // The binding's profile name is never echoed: the
                    // allowlist is also the client's information boundary —
                    // `list_profiles` never names a profile outside it.
                    Err("profile not available".to_owned())
                }
                _ => Ok(()),
            }
        }
    }
}

/// The saved binding's profile: `Ok(Some)` when a binding reads cleanly,
/// `Ok(None)` when no binding exists — the run command's own refusal
/// applies — and `Err` when the binding cannot be read, which the gate
/// fails closed on.
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
