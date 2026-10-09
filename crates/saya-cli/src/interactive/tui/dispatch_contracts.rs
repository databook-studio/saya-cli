//! TUI adapters for the contract slash commands.
//!
//! The TUI runs under the alternate screen, so the shared headless
//! `run_contracts` dispatcher — which `emit`s to a thread-local
//! seam — is wrapped here: the session's active profile is stamped onto the
//! command (the same field the headless `--profile` flag sets, not a second
//! resolution or a second privacy decision), the dispatcher runs, and its
//! captured output is pushed into the transcript as a system or error block.
//!
//! Extracted from `dispatch_actions.rs` to keep that file under the size cap;
//! the seam that routes a command at the active profile lives here, with its
//! tests.

use super::transcript::{BlockKind, Transcript};
use crate::cli::ContractsCommand;
use crate::commands::{
    capture_output_start, capture_output_take, run_contracts as run_contracts_command,
};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::lifecycle::resume::block_on;
use crate::interactive::session::state::SessionState;
use crate::render::RenderFormat;

/// Runs a contract slash command through the shared `run_contracts` dispatcher
/// and pushes its rendered output into the transcript. The TUI runs under the
/// alternate screen, so `run_contracts`'s `emit` output is captured through the
/// thread-local seam instead of going to the process stdout; a non-zero exit
/// surfaces as an error block.
pub(super) fn run_contracts(
    transcript: &mut Transcript,
    state: &SessionState,
    runtime: &RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
    format: RenderFormat,
    command: &ContractsCommand,
) {
    // The session's selected profile maps to the headless `--profile` field:
    // setting it (not a second resolution) routes the command at the database
    // the user /connect-ed to, matching every other TUI slash command.
    let command = with_profile(command, state.profile.as_deref());
    capture_output_start();
    // The shared dispatcher returns Ok(code) for every typed outcome (a store
    // error is emitted as a diagnostic and returned non-zero); the outer Err is
    // a render/IO failure, surfaced here as an error block.
    let code = match block_on(run_contracts_command(command, runtime, format, state_db)) {
        Ok(code) => code,
        Err(error) => {
            capture_output_take();
            transcript.push(BlockKind::Error, error.to_string());
            return;
        }
    };
    let (out, err) = capture_output_take();
    let body = if out.trim().is_empty() { err } else { out };
    if code == 0 {
        transcript.push(BlockKind::System, body.trim_end().to_string());
    } else {
        transcript.push(BlockKind::Error, body.trim_end().to_string());
    }
}

/// Sets the `profile` field on a `ContractsCommand` to the session's active
/// profile. This is the same field the headless `--profile` flag sets; it is
/// not a second resolution or a second privacy decision.
///
/// The match is exhaustive on purpose: every variant is named, so a future
/// profile-bearing variant the slash path routes through the TUI is a compile
/// error here, not a silent fall-through that reads the configured default's
/// data. `Forget` has no `profile` field — it addresses a claim by id — so it
/// passes through unchanged.
fn with_profile(command: &ContractsCommand, profile: Option<&str>) -> ContractsCommand {
    let profile = profile.map(str::to_string);
    match command {
        ContractsCommand::List { .. } => ContractsCommand::List { profile },
        ContractsCommand::Show { table, .. } => ContractsCommand::Show {
            table: table.clone(),
            profile,
        },
        ContractsCommand::Queue { limit, .. } => ContractsCommand::Queue {
            profile,
            limit: *limit,
        },
        ContractsCommand::Remember {
            table,
            kind,
            value,
            column,
            reason,
            ..
        } => ContractsCommand::Remember {
            table: table.clone(),
            kind: *kind,
            value: value.clone(),
            column: column.clone(),
            reason: reason.clone(),
            profile,
        },
        // `Decide` carries a `profile` field like the other profiled
        // reads/writes: the TUI stamps the session's active profile so a
        // `/confirm ki-xxxx` resolves against the database the user /connect-ed
        // to, not the configured default (cross-profile isolation, the same
        // invariant `/queue`'s stamp upholds).
        ContractsCommand::Decide {
            prefix, decision, ..
        } => ContractsCommand::Decide {
            prefix: prefix.clone(),
            decision: *decision,
            profile,
        },
        // the batch approve is a profiled command like `/queue` — the
        // session's active profile is stamped so `/approve-all` approves the
        // queue of the database the user is looking at, never the configured
        // default's.
        ContractsCommand::ApproveAll { limit, yes, .. } => ContractsCommand::ApproveAll {
            profile,
            limit: *limit,
            yes: *yes,
        },
        ContractsCommand::Forget { claim_id, reason } => ContractsCommand::Forget {
            claim_id: claim_id.clone(),
            reason: *reason,
        },
        // The portable context operations (B2c) are profiled like the reads
        // above: the session's active profile is stamped so the export reads
        // and the import writes the database the user /connect-ed to.
        ContractsCommand::Export {
            path, overwrite, ..
        } => ContractsCommand::Export {
            path: path.clone(),
            profile,
            overwrite: *overwrite,
        },
        ContractsCommand::Import { path, preview, .. } => ContractsCommand::Import {
            path: path.clone(),
            profile,
            preview: *preview,
        },
        ContractsCommand::ImportDbt {
            manifest,
            select,
            preview,
            ..
        } => ContractsCommand::ImportDbt {
            manifest: manifest.clone(),
            profile,
            select: select.clone(),
            preview: *preview,
        },
    }
}

#[cfg(test)]
#[path = "dispatch_contracts_tests.rs"]
mod tests;
