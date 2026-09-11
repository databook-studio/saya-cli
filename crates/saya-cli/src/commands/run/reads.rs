//! `saya run list | show | log` — the read surfaces over the run mirror and
//! the run directories.
//!
//! The store is the metadata mirror (`saya run list` reads it); a run's
//! goal, scopes, and budget live in its spec file, and its history in its
//! journal. Unknown ids fail cleanly with the contract-command precedent's
//! usage code — never a panic, never an ignored scope.

use std::collections::BTreeMap;

use super::{parse_run_id, runs_dir};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use crate::render_run;
use crate::render_usage;
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
    // The spec is the approval's full shape; a missing file renders as its
    // own line rather than failing the read. The stanza's wording is the
    // renderer's (`crate::render_run`) — the headless command and the `/runs`
    // slash adapter render the same bytes because both end here.
    let spec = super::files::load_spec(&dir).ok().map(|spec| {
        let scopes = describe_scopes(&spec.scopes);
        (spec.goal, scopes)
    });
    let journal = journal_of(&dir)?;
    let paused = super::last_pause(&journal);
    let events = journal
        .read()
        .map_err(|error| format!("run journal could not be read: {error}"))?;
    let deliverables = deliverable_lines(&events);
    let usage = render_usage::usage_by_endpoint(&events);
    let text = render_run::run_show_text(render_run::RunShowStanza {
        id: record.id.as_str(),
        status: record.status.as_str(),
        failure_cause: match (record.status, record.failure_code) {
            (saya_store::RunStatus::Failed, Some(code)) => {
                Some(super::exit::failure_code_cause(code))
            }
            _ => None,
        },
        created_unix_ms: record.created_unix_ms,
        updated_unix_ms: record.updated_unix_ms,
        spec: spec
            .as_ref()
            .map(|(goal, scopes)| (goal.as_str(), scopes.as_str())),
        paused,
        deliverables: &deliverables,
        usage: &usage,
    });
    crate::commands::output::result(text, format)
}

/// Prints one run's journal: every event, in write order, one line each.
/// Text renders the shaper's own line per event (`crate::render_run`), so a
/// reader sees what happened rather than wire bytes; JSON and NDJSON keep
/// the journal's bytes — one `journal_line` per event, the same bytes the
/// live run wire streams and the only framing a script should parse. An
/// event the shaper does not know (a future variant) still shows — its one
/// journal line — rather than vanishing from the record it belongs to.
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
    let lines = match format {
        // Each line already carries its newline; the result envelope adds
        // the final one, so the block is trimmed to one line per event.
        RenderFormat::Text => events
            .iter()
            .map(log_text_line)
            .collect::<String>()
            .trim_end_matches('\n')
            .to_string(),
        RenderFormat::Json | RenderFormat::Ndjson => events
            .iter()
            .map(render_run::journal_line)
            .collect::<Vec<_>>()
            .join("\n"),
    };
    crate::commands::output::result(lines, format)
}

/// One journal event's text line for `run log`. The shaper's line when it
/// has one; the journal's own bytes when the shaper does not know the
/// variant — the log shows every recorded event, never hides one.
fn log_text_line(event: &RunEvent) -> String {
    let text = render_run::run_event_text(event);
    if text.is_empty() {
        format!("{}\n", render_run::journal_line(event))
    } else {
        text
    }
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
