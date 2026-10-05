//! Deterministic continuation facts: bounded observations of what the
//! session's tools actually did, gathered turn by turn from live output, and
//! the pure builder that turns them into the contract's FACT part. Model
//! prose never feeds either — a forged answer mints no fact.

mod checks;
#[allow(dead_code)]
// the D04 seam: nothing wires the fact builder until the compaction writer lands
mod facts;

use saya_agent::{AgentOutput, ToolMetadata};
use saya_types::{CheckOutcome, MAX_FACT_PATH_BYTES, MAX_FILE_FACTS};

pub(crate) use facts::now_unix_ms;

use checks::absorb_checks;

/// Tool-call arguments past this size are never parsed: a bounded collector
/// reads bounded inputs.
const MAX_ARGUMENT_BYTES: usize = 64 * 1024;

/// One observed file touch: the relative workspace path, the tool that
/// touched it, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileObservation {
    pub(crate) path: String,
    pub(crate) tool: String,
    pub(crate) observed_unix_ms: i64,
}

/// One observed check: the redacted one-line command, how it ended, when,
/// and which tool ran it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckObservation {
    pub(crate) label: String,
    pub(crate) outcome: CheckOutcome,
    pub(crate) tool: String,
    pub(crate) observed_unix_ms: i64,
}

/// Bounded observations of actual tool activity, gathered live and never
/// persisted: the raw material [`build_continuation_facts`] turns into
/// contract facts. Files dedupe by path with the latest touch winning; both
/// collections evict their oldest past the contract's fact bounds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SessionObservations {
    pub(crate) files: Vec<FileObservation>,
    pub(crate) checks: Vec<CheckObservation>,
}

impl SessionObservations {
    /// Absorbs one live turn output: completed write-shaped tool calls
    /// become file observations, and `run_command` completion events whose
    /// summary parses to an exit code become checks. The assistant answer is
    /// never read — prose mints nothing.
    pub(crate) fn absorb(&mut self, output: &AgentOutput, now_unix_ms: i64) {
        for tool in &output.tool_metadata {
            if tool.status == "completed"
                && matches!(tool.name.as_str(), "workspace_edit" | "workspace_write")
                && let Some(path) = completed_path(tool)
            {
                self.record_file(FileObservation {
                    path,
                    tool: tool.name.clone(),
                    observed_unix_ms: now_unix_ms,
                });
            }
        }
        absorb_checks(&mut self.checks, &output.events, now_unix_ms);
    }

    /// Upserts a file observation: deduped by path, latest touch last,
    /// oldest evicted past the contract's file-fact bound.
    fn record_file(&mut self, observation: FileObservation) {
        self.files.retain(|file| file.path != observation.path);
        self.files.push(observation);
        if self.files.len() > MAX_FILE_FACTS {
            self.files.remove(0);
        }
    }
}

/// A completed write-shaped tool call names its file: the bounded arguments
/// are parsed and the `path` is kept only when it passes the contract's
/// relative-path rule, so an invalid observation never reaches a fact.
fn completed_path(tool: &ToolMetadata) -> Option<String> {
    if tool.arguments.len() > MAX_ARGUMENT_BYTES {
        return None;
    }
    let arguments: serde_json::Value = serde_json::from_str(&tool.arguments).ok()?;
    let path = arguments.get("path")?.as_str()?;
    is_fact_path(path).then(|| path.to_owned())
}

/// The contract's relative-path rule, mirrored here because the contract's
/// own check is private: non-empty, bounded, no control characters, not
/// absolute on any platform, and no empty or `..` components on either
/// separator.
fn is_fact_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_FACT_PATH_BYTES
        && !path.chars().any(char::is_control)
        && !is_absolute(path)
        && !path
            .split(['/', '\\'])
            .any(|component| component.is_empty() || component == "..")
}

/// Absolute on any platform: a leading separator, or a Windows drive root.
fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.first() == Some(&b'/')
        || bytes.first() == Some(&b'\\')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'/' || bytes[2] == b'\\'))
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod record_tests;
