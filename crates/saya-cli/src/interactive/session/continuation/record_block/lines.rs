//! The record block's rendered lines: section assembly with the visible
//! budget cut, and the per-fact lines — file drift refreshed from the bound
//! workspace, checks labelled historical.

use saya_harness::workspace::Workspace;
use saya_types::{CheckOutcome, FileFact, redact};

use super::MAX_REFRESHED_FILES;
use super::age::age_suffix;

/// One rendered section: its heading, an optional prelude line, and the
/// droppable fact lines. A section that loses its last line leaves with its
/// heading.
pub(super) struct Section {
    pub(super) heading: &'static str,
    pub(super) prelude: Option<&'static str>,
    pub(super) lines: Vec<String>,
}

/// The rendered body: the fixed header, every section that still has lines,
/// and the visible cut note when lines were dropped.
pub(super) fn body_of(sections: &[Section], dropped: usize) -> String {
    let mut pieces: Vec<String> = vec![super::HEADER.to_owned()];
    for section in sections {
        if section.lines.is_empty() {
            continue;
        }
        pieces.push(section.heading.to_owned());
        if let Some(prelude) = section.prelude {
            pieces.push(prelude.to_owned());
        }
        pieces.extend(section.lines.iter().cloned());
    }
    if dropped > 0 {
        pieces.push(format!("… {dropped} more not shown"));
    }
    pieces.join("\n")
}

/// One file line: the recorded path, the status refreshed from the bound
/// workspace, and the coarse age of the observation.
pub(super) fn file_line(
    fact: &FileFact,
    workspace: Option<&Workspace>,
    index: usize,
    now: i64,
) -> String {
    format!(
        "- {} — {} {}",
        fact.path,
        file_status(fact, workspace, index),
        age_suffix(fact.observed_unix_ms, now),
    )
}

/// The drift status: a digest-absent fact never reads; the first
/// `MAX_REFRESHED_FILES` facts re-read their file through the contained
/// workspace read (the whole-file digest, no content materialised); the rest
/// keep their recorded shape unverified.
fn file_status(fact: &FileFact, workspace: Option<&Workspace>, index: usize) -> &'static str {
    let Some(recorded) = fact.sha256.as_deref() else {
        return "digest not recorded";
    };
    let Some(workspace) = workspace else {
        return "not re-checked";
    };
    if index >= MAX_REFRESHED_FILES {
        return "not re-checked";
    }
    match workspace.read(&fact.path, 0) {
        Ok(file) if file.digest == recorded => "unchanged since observed",
        Ok(_) => "changed since observed",
        Err(error) if error.is_not_found() => "not found",
        Err(_) => "unreadable",
    }
}

/// One check line: the redacted label, the recorded outcome's serde name, and
/// the coarse age — historical data, never re-run here.
pub(super) fn check_line(fact: &saya_types::CheckFact, now: i64) -> String {
    format!(
        "- {} — {} {}",
        redact(&fact.label),
        outcome_name(fact.outcome),
        age_suffix(fact.observed_unix_ms, now),
    )
}

/// The outcome's existing serde name (`rename_all = "snake_case"`); a newer
/// binary's outcome prints unknown rather than inventing a name.
fn outcome_name(outcome: CheckOutcome) -> &'static str {
    match outcome {
        CheckOutcome::Passed => "passed",
        CheckOutcome::Failed => "failed",
        CheckOutcome::TimedOut => "timed_out",
        _ => "unknown",
    }
}
