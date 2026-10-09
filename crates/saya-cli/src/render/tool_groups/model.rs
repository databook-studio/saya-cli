/// One request/completion pair inside a group.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolCallPair {
    pub(crate) name: String,
    pub(crate) arguments: serde_json::Value,
    pub(crate) effect: Option<ToolEffect>,
    pub(crate) summary: Option<String>,
    pub(crate) failed: bool,
    pub(crate) nonzero_exit: bool,
}

/// A maximal run of tool events between two content boundaries.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolGroup {
    pub(crate) calls: Vec<ToolCallPair>,
}

/// Failure is the existing contract: the summary keeps the substring
/// "failed", which is what `tool_metadata.status` derives from. This keys on
/// the same predicate, never a second definition.
pub(crate) fn is_failure_summary(summary: &str) -> bool {
    summary.contains("failed")
}

/// The live per-call completion line both TUI render sites mirror onto the
/// tail: `✓` for a completion that succeeded, `✗` for one that failed.
/// Keys on [`is_failure_summary`], never a second definition, so the live
/// tail and the flushed group detail cannot drift — and so a future change
/// to the failure predicate moves both marks together.
pub(crate) fn live_completion_line(name: &str, summary: &str) -> String {
    let mark = if is_failure_summary(summary) {
        "✗"
    } else {
        "✓"
    };
    format!("{mark} {name}: {summary}")
}

/// A nonzero host exit as reported on the piped surface: an `Ok` outcome
/// whose completion summary reads `<program> exited <nonzero>` — no
/// "failed" substring, so `is_failure_summary` (and `tool_metadata.status`,
/// which derives from the same predicate) still reads it as completed. The
/// group shaper consults this separately so the exit code stays visible on
/// the default surface without changing the "failed" contract.
pub(crate) fn is_nonzero_exit_summary(summary: &str) -> bool {
    run_command_exit_code(summary).is_some_and(|code| code != 0)
}

/// The exit code a `run_command` completion summary carries, when it carries
/// one: `<program> exited <code>`. `None` for every other summary shape —
/// including the failure arm (`failed <program>`), which names the call but
/// never the code.
fn run_command_exit_code(summary: &str) -> Option<i64> {
    let (_, code) = summary.rsplit_once(" exited ")?;
    code.parse().ok()
}
