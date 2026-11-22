//! The consent line a job's start puts to the operator, as both surfaces show
//! it.
//!
//! A `/start` proposes one job and waits for an explicit yes. The proposal is a
//! JSON object — exact, and unreadable on a terminal — so the line first states
//! the facts the operator decides on: which job, the task it was given, the
//! paths it may write, and the identity of every database profile it may read.
//! The proposal itself follows verbatim on the last line, so the exact bytes
//! this yes covers are still there to be read.
//!
//! Nothing here decides anything. Today's panel writes this line into its
//! transcript and the new surface paints the same line on its consent card, so
//! one wording serves both and neither invents a fact of its own.

use serde_json::Value;

/// The longest consent line this states, in bytes: the proposal is bounded
/// where it is built, and a line longer than this is not a fact an operator
/// decided on.
const MAX_BYTES: usize = 16 * 1024;

/// The consent text for `id`'s proposal `arguments`.
pub(super) fn text(id: &str, arguments: &Value) -> String {
    let mut rows = vec![format!("Approve job {id} with this exact scope:")];
    if let Some(task) = arguments.get("task").and_then(Value::as_str) {
        rows.push(format!("task: {task}"));
    }
    if let Some(paths) = write_paths(arguments) {
        rows.push(format!("write: {}", paths.join(", ")));
    }
    for (name, identity) in profiles(arguments) {
        rows.push(format!("\"{name}\" {}", stated(&identity)));
    }
    rows.push(format!("Use `/job approve {id}` or `/job deny {id}`."));
    rows.push(arguments.to_string());
    bounded(rows.join("\n"))
}

/// The paths this proposal would let the job write. An in-place proposal states
/// them as its write scope; a Git job's proposal states none, because its edits
/// stay in its own checkout.
fn write_paths(arguments: &Value) -> Option<Vec<String>> {
    let stated = arguments
        .get("write_scope")
        .or_else(|| arguments.get("write_paths"))?;
    let paths: Vec<String> = stated
        .as_array()?
        .iter()
        .filter_map(|path| path.as_str())
        .map(str::to_owned)
        .collect();
    (!paths.is_empty()).then_some(paths)
}

/// The database profiles this proposal resolved, in the order it states them:
/// each name and the identity its profile was resolved to.
fn profiles(arguments: &Value) -> Vec<(String, Value)> {
    arguments
        .get("database_profiles")
        .and_then(Value::as_object)
        .map(|profiles| {
            profiles
                .iter()
                .map(|(name, identity)| (name.clone(), identity.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// One identity as the proposal states it.
fn stated(identity: &Value) -> String {
    identity
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| identity.to_string())
}

/// `text` cut to the bound on a character boundary: a row is never half a
/// character.
fn bounded(text: String) -> String {
    if text.len() <= MAX_BYTES {
        return text;
    }
    let mut end = MAX_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
#[path = "consent_tests.rs"]
mod tests;
