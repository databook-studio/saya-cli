//! TUI adapter for the `/investigation` slash commands.
//!
//! Mirrors `dispatch_contracts.rs`: the shared headless `run_investigation`
//! dispatcher — which `emit`s to a thread-local seam — is wrapped here, its
//! captured output pushed into the transcript as a system or error block. The
//! TUI never reads stdin, so the adapter passes `can_prompt = false` — the
//! same stdin answer every other TUI door gives.
//!
//! The TUI-only behaviour is the save-without-SQL fill: `/investigation save
//! <name>` with neither `--sql` nor `--file` saves the latest successful,
//! concrete query on the connection that actually ran it. An explicit `--sql`
//! or `--file` always wins; the current profile is never substituted
//! silently.

use super::transcript::{BlockKind, Transcript};
use super::types::LastQuery;
use crate::cli::InvestigationCommand;
use crate::commands::{
    capture_output_start, capture_output_take, run_investigation as run_investigation_command,
};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_resume::block_on;
use crate::render::RenderFormat;

/// Fills a save that carries neither `--sql` nor `--file` from the session's
/// last selectable query: the SQL it ran and the connection that ran it.
/// Everything else passes through unchanged.
pub(super) fn fill_save_from_last_query(
    command: InvestigationCommand,
    last_query: &Option<LastQuery>,
) -> Result<InvestigationCommand, String> {
    let InvestigationCommand::Save {
        name,
        description,
        sql,
        file,
        connection,
    } = command
    else {
        return Ok(command);
    };
    // An explicit `--sql` / `--file` always wins: the last query is left out
    // entirely, its connection included.
    if sql.is_some() || file.is_some() {
        return Ok(InvestigationCommand::Save {
            name,
            description,
            sql,
            file,
            connection,
        });
    }
    let Some(last) = last_query else {
        return Err("Nothing to save yet: run a query first (or pass --sql).".into());
    };
    let Some(connection) = connection.or_else(|| last.connection.clone()) else {
        return Err(
            "The last query did not name its connection; re-run it with /sql, or pass \
             --connection."
                .into(),
        );
    };
    Ok(InvestigationCommand::Save {
        name,
        description,
        sql: Some(last.sql.clone()),
        file: None,
        connection: Some(connection),
    })
}

/// Runs one investigation command through the shared `run_investigation`
/// dispatcher and pushes its rendered output into the transcript. The TUI
/// runs under the alternate screen, so the dispatcher's `emit` output is
/// captured through the thread-local seam instead of going to the process
/// stdout; a non-zero exit surfaces as an error block.
pub(super) fn run_investigation(
    transcript: &mut Transcript,
    runtime: &RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
    format: RenderFormat,
    command: &InvestigationCommand,
    last_query: &Option<LastQuery>,
) {
    let command = match fill_save_from_last_query(command.clone(), last_query) {
        Ok(command) => command,
        Err(message) => {
            transcript.push(BlockKind::Error, message);
            return;
        }
    };
    capture_output_start();
    // The shared dispatcher returns Ok(code) for every typed outcome (a store
    // error is emitted as a diagnostic and returned non-zero); the outer Err
    // is a render/IO failure, surfaced here as an error block.
    let code = match block_on(run_investigation_command(
        command, runtime, format,
        // The TUI's stdin answer: it never reads stdin, so a missing secret
        // surfaces as an error rather than a stdin prompt.
        false, state_db,
    )) {
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

#[cfg(test)]
#[path = "dispatch_investigation_tests.rs"]
mod tests;
