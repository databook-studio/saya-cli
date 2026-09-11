//! `saya run list | show | log` — the read surfaces over the run mirror and
//! the run directories.
//!
//! The store is the metadata mirror (`saya run list` reads it); a run's
//! goal, scopes, and budget live in its spec file, and its history in its
//! journal. Unknown ids fail cleanly with the contract-command precedent's
//! usage code — never a panic, never an ignored scope.

use std::collections::BTreeMap;

use super::exit::pause_reason_text;
use super::{parse_run_id, runs_dir};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_store::{RunStore, SqliteStateStore};
use saya_types::{Deliverable, RunEvent};

pub(super) async fn list(
    _runtime: &RuntimeConfig,
    format: RenderFormat,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let runs = RunStore::list_runs(state)
        .await
        .map_err(|error| format!("run state store refused the listing: {error}"))?;
    if runs.is_empty() {
        return crate::commands::output::result("no runs yet".to_string(), format);
    }
    let lines = runs
        .iter()
        .map(|row| {
            format!(
                "{}\t{}\tcreated {}\tupdated {}",
                row.id,
                row.status.as_str(),
                row.created_unix_ms,
                row.updated_unix_ms
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    crate::commands::output::result(lines, format)
}

/// Loads the journal of a run known to the store; an unreadable journal is a
/// connection/config-class failure, not a panic.
fn journal_of(dir: &std::path::Path) -> Result<saya_harness::journal::Journal, String> {
    let journal = saya_harness::journal::Journal::open(dir);
    journal
        .read()
        .map(|_| journal)
        .map_err(|error| format!("run journal could not be read: {error}"))
}

pub(super) async fn show(
    raw_id: &str,
    _runtime: &RuntimeConfig,
    format: RenderFormat,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let run_id = parse_run_id(raw_id)?;
    let dir = runs_dir().join(run_id.as_str());
    let Some(record) = RunStore::get_run(state, &run_id)
        .await
        .map_err(|error| format!("run state store refused the lookup: {error}"))?
    else {
        return crate::commands::output::failure_message(
            2,
            format!("no run with id {run_id}"),
            format,
        );
    };
    let mut text = format!(
        "run {}\nstatus: {}{}\ncreated: {}\nupdated: {}",
        record.id,
        record.status.as_str(),
        match (record.status, record.failure_code) {
            (saya_store::RunStatus::Failed, Some(code)) => {
                format!(" ({})", super::exit::failure_code_cause(code))
            }
            _ => String::new(),
        },
        record.created_unix_ms,
        record.updated_unix_ms,
    );
    // The spec is the approval's full shape; a missing file renders as its
    // own line rather than failing the read.
    if let Ok(spec) = super::files::load_spec(&dir) {
        text.push_str(&format!("\ngoal: {}", spec.goal));
        text.push_str(&format!("\nscopes: {}", describe_scopes(&spec.scopes)));
    }
    let journal = journal_of(&dir)?;
    if let Some(reason) = super::last_pause(&journal) {
        text.push_str(&format!("\npaused: {}", pause_reason_text(reason)));
    }
    // The deliverables the run recorded at its steps' completions — the
    // artifact manifests, last record per step. A run that declared none
    // renders no section.
    let events = journal
        .read()
        .map_err(|error| format!("run journal could not be read: {error}"))?;
    let lines = deliverable_lines(&events);
    if !lines.is_empty() {
        text.push_str("\ndeliverables:");
        for line in lines {
            text.push_str(&format!("\n{line}"));
        }
    }
    crate::commands::output::result(text, format)
}

/// Prints one run's journal: every lifecycle and step event, in write order,
/// one NDJSON line each — the journal is already the wire format.
pub(super) async fn log(
    raw_id: &str,
    _runtime: &RuntimeConfig,
    format: RenderFormat,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let run_id = parse_run_id(raw_id)?;
    let dir = runs_dir().join(run_id.as_str());
    let Some(_) = RunStore::get_run(state, &run_id)
        .await
        .map_err(|error| format!("run state store refused the lookup: {error}"))?
    else {
        return crate::commands::output::failure_message(
            2,
            format!("no run with id {run_id}"),
            format,
        );
    };
    let journal = match journal_of(&dir) {
        Ok(journal) => journal,
        Err(message) => return crate::commands::output::failure_message(3, message, format),
    };
    let events = journal
        .read()
        .map_err(|error| format!("run journal could not be read: {error}"))?;
    if events.is_empty() {
        return crate::commands::output::result(
            format!("run {run_id} has no journal events"),
            format,
        );
    }
    let lines = events
        .iter()
        .map(|event| serde_json::to_string(event).unwrap_or_else(|_| format!("{event:?}")))
        .collect::<Vec<_>>()
        .join("\n");
    crate::commands::output::result(lines, format)
}

/// The run's recorded deliverables — the last manifest per step, in step
/// order: name, size, digest. A declared deliverable the step never
/// produced is rendered missing, never silently dropped.
fn deliverable_lines(events: &[RunEvent]) -> Vec<String> {
    // Last record wins: a step re-driven by a resume appends a fresh
    // manifest for its step.
    let mut by_step: BTreeMap<usize, Vec<&Deliverable>> = BTreeMap::new();
    for event in events {
        if let RunEvent::Deliverables { step, entries } = event {
            by_step.insert(*step, entries.iter().collect());
        }
    }
    let mut lines = Vec::new();
    for (step, entries) in &by_step {
        for deliverable in entries {
            match &deliverable.artifact {
                Some(artifact) => lines.push(format!(
                    "step {step}: {} ({} bytes, sha256 {})",
                    deliverable.name, artifact.size, artifact.digest
                )),
                None => lines.push(format!("step {step}: {} missing", deliverable.name)),
            }
        }
    }
    lines
}

/// The scopes as the user declared them: booleans and named sets.
fn describe_scopes(caps: &saya_types::Capabilities) -> String {
    let mut names = Vec::new();
    if caps.workspace_write {
        names.push("workspace-write".to_string());
    }
    if caps.scratch {
        names.push("scratch".to_string());
    }
    if let Some(fetch) = &caps.fetch {
        for destination in &fetch.destinations {
            names.push(format!("fetch:{}+{}", destination.scheme, destination.host));
        }
    }
    if let Some(runner) = &caps.runner {
        for program in &runner.programs {
            names.push(format!("runner:{program}"));
        }
    }
    for (role, endpoint) in caps.endpoints.as_map() {
        names.push(format!("endpoint:{role}={endpoint}"));
    }
    if names.is_empty() {
        "(none)".to_string()
    } else {
        names.join(", ")
    }
}
