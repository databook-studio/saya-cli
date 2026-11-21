//! The step line of an activity block: one short human phrase per tool call,
//! and the block's folded summary.
//!
//! Strings only — no glyph, colour or widget — so the pane above decides how a
//! line is painted and a test can assert the words without a terminal.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::entry::{Step, StepState};

/// One step read as a short phrase: the tool's verb and the fact the operator
/// needs to tell the call apart.
pub(crate) fn step_line(step: &Step, root: Option<&Path>) -> String {
    let arguments = &step.arguments;
    let line = match step.tool.as_str() {
        "schema_discovery" => "Read schema".to_owned(),
        "bounded_sql_query" => match text(arguments, "connection") {
            Some(connection) => format!("Query {connection}"),
            None => "Query database".to_owned(),
        },
        "bounded_sql_query_all" => "Query all databases".to_owned(),
        "result_shape" | "column_health" | "join_check" => "Check data".to_owned(),
        "contract_search" | "contract_read" => "Recall".to_owned(),
        "designate_answer" => "Mark answer".to_owned(),
        "read" => named("Read", text(arguments, "path")),
        "list" | "glob" | "grep" => "Search".to_owned(),
        "write" | "edit" => named("Edit", text(arguments, "path")),
        "shell" => named("Run", own_words(text(arguments, "command"), root)),
        "web_fetch" => named("Fetch", text(arguments, "url").map(host)),
        "web_search" => "Search web".to_owned(),
        "helpers" => named("Helper", helper_id(arguments)),
        other => other.to_owned(),
    };
    match settled_rows(step) {
        Some(rows) => format!("{line} · {rows}"),
        None => line,
    }
}

/// The folded summary of an activity block: how many steps, what kinds, and
/// how long the block took.
pub(crate) fn folded_summary(steps: &[Step], elapsed: Duration) -> String {
    let mut parts = vec![counted(steps.len(), "step", "steps")];
    let kinds = kinds(steps);
    if !kinds.is_empty() {
        parts.push(kinds.join(", "));
    }
    parts.push(elapsed_text(elapsed));
    parts.join(" · ")
}

/// The rows a settled query returned — the one outcome a step line carries,
/// because it is the fact the operator waits to see.
fn settled_rows(step: &Step) -> Option<&str> {
    let StepState::Done { summary } = &step.state else {
        return None;
    };
    let query = matches!(
        step.tool.as_str(),
        "bounded_sql_query" | "bounded_sql_query_all"
    );
    query
        .then_some(summary.as_str())
        .filter(|rows| !rows.is_empty())
}

/// The string a call carries under `key`, absent and empty reading the same.
fn text<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// `verb` followed by the call's own words, or the verb alone when the call
/// named nothing.
fn named(verb: &str, words: Option<&str>) -> String {
    match words {
        Some(words) => format!("{verb} {words}"),
        None => verb.to_owned(),
    }
}

/// The host of a URL, without its scheme, query or path.
fn host(url: &str) -> &str {
    let authority = url.split_once("://").map_or(url, |(_, rest)| rest);
    authority.split(['/', '?', '#']).next().unwrap_or(authority)
}

/// The name a helper call carries: its own description, else the agent id it
/// named.
fn helper_id(arguments: &Value) -> Option<&str> {
    text(arguments, "description").or_else(|| text(arguments, "agent_id"))
}

/// A command as the operator reads it: the `cd` that only exists to reach the
/// workspace root is not part of the command.
fn own_words<'a>(command: Option<&'a str>, root: Option<&Path>) -> Option<&'a str> {
    let command = command?;
    Some(strip_root_cd(command, root).unwrap_or(command))
}

/// `command` without a leading `cd <root> &&`, when that `cd` reaches exactly
/// the workspace root. A `cd` anywhere else is part of the command and stays.
fn strip_root_cd<'a>(command: &'a str, root: Option<&Path>) -> Option<&'a str> {
    let root = root?;
    let rest = command.strip_prefix("cd ")?;
    let (directory, rest) = rest.split_once("&&")?;
    let into_root = Path::new(directory.trim().trim_matches(['\'', '"'])) == root;
    into_root.then(|| rest.trim_start())
}

/// How one kind of step reads in the folded line: its words singular and
/// plural, and whether the line states how many.
fn kind_words(tool: &str) -> (String, String, bool) {
    let (one, many) = match tool {
        "schema_discovery" => ("read schema", "read schemas"),
        "bounded_sql_query" | "bounded_sql_query_all" => ("query", "queries"),
        "result_shape" | "column_health" | "join_check" => ("check", "checks"),
        "contract_search" | "contract_read" => ("recall", "recalls"),
        "designate_answer" => ("mark answer", "mark answers"),
        "read" => ("read", "reads"),
        "list" | "glob" | "grep" => ("search", "searches"),
        "write" | "edit" => ("edit", "edits"),
        "shell" => ("command", "commands"),
        "web_fetch" => ("fetch", "fetches"),
        "web_search" => ("web search", "web searches"),
        "helpers" => ("helper", "helpers"),
        other => return (other.to_owned(), format!("{other}s"), true),
    };
    (one.to_owned(), many.to_owned(), tool != "schema_discovery")
}

/// The block's kinds in the order the block first reached them, each spelled
/// with the count it holds.
fn kinds(steps: &[Step]) -> Vec<String> {
    let mut held: Vec<(String, String, bool, usize)> = Vec::new();
    for step in steps {
        let (one, many, with_count) = kind_words(&step.tool);
        match held.iter_mut().find(|(name, ..)| *name == one) {
            Some((_, _, _, count)) => *count += 1,
            None => held.push((one, many, with_count, 1)),
        }
    }
    held.into_iter()
        .map(|(one, many, with_count, count)| match (with_count, count) {
            (true, count) => counted(count, &one, &many),
            (false, 1) => one,
            (false, _) => many,
        })
        .collect()
}

/// `count` with its word in the number the count takes.
fn counted(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// The block's time: seconds under a minute, minutes and seconds above it.
fn elapsed_text(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {}s", seconds / 60, seconds % 60)
    }
}
