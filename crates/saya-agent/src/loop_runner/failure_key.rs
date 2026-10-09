//! The key a tool call is remembered under in the run's failure memory.
//!
//! Two kinds of call share the one bounded failure set (see
//! `failed_statements.rs`), but they are not keyed the same way, and the
//! difference is deliberate:
//!
//! - A call carrying a `sql` string keys on that statement **exactly as
//!   submitted** — no normalisation of any kind, and nothing else about the
//!   call participates. The failure lives in the statement, so a different
//!   `connection` argument must not make a known-bad statement look fresh;
//!   and a model that edits the SQL at all has changed its approach and must
//!   not be refused.
//! - Every other tool keys on the tool name plus its arguments: the whole
//!   invocation is the failure, so the same program with the same arguments
//!   is refused, while different arguments are a genuine change of approach
//!   and must execute.
//!
//! The one thing that *is* normalised here is the serialization of a non-SQL
//! call's arguments, which is not a contradiction of the rule above:
//! `serde_json`'s object key order is a construction detail of the value, not
//! part of what the model submitted, so two semantically identical calls must
//! render to one key. What stays un-normalised is the *content* — a model
//! that edits an argument value has changed its approach.

use super::failed_statements::sql_of;
use super::tools::floor_boundary;
use crate::ToolCall;

/// The bound on a remembered key's rendered arguments. A degenerate run may
/// submit enormous arguments; the entry-count FIFO cap alone would still let
/// memory grow with the size of each entry. Keys whose rendering exceeds this
/// keep a prefix of the rendering plus a 64-bit FNV-1a hash of the *whole*
/// rendering, so identical huge calls still collide and distinct ones collide
/// only by accident (≈2⁻⁶⁴ per pair, on top of having to share the prefix).
/// A deliberately crafted collision is not a threat: a model that crafts one
/// only fools itself into a refusal it could have had by repeating verbatim.
const MAX_ARGUMENTS_BYTES: usize = 512;

/// The key a call is remembered under. The variants keep a SQL statement and
/// a tool invocation from colliding by spelling: a `run_program` whose
/// arguments happen to render to the text of a failed statement is not that
/// statement's repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CallKey {
    /// A SQL statement, exactly as submitted.
    Sql(String),
    /// A non-SQL tool call: the tool name, then the arguments rendered
    /// canonically (object keys sorted recursively) and bounded at
    /// [`MAX_ARGUMENTS_BYTES`].
    ToolCall(String, String),
}

/// The key `call` is remembered under in the failure memory: the SQL path
/// ([`sql_of`]) when the call carries a `sql` string, the general
/// (tool, arguments) path beside it otherwise.
pub(super) fn key_of(call: &ToolCall) -> CallKey {
    if let Some(sql) = sql_of(call) {
        return CallKey::Sql(sql.to_owned());
    }
    CallKey::ToolCall(call.name.clone(), bounded_arguments(&call.arguments))
}

/// The canonical, bounded rendering of `arguments` for a key: sorted object
/// keys recursively ([`canonical_json`]), and past [`MAX_ARGUMENTS_BYTES`] a
/// prefix of the rendering plus a hash of the whole thing — memory stays
/// bounded even for a degenerate run with enormous arguments.
fn bounded_arguments(arguments: &serde_json::Value) -> String {
    let rendered = canonical_json(arguments);
    if rendered.len() <= MAX_ARGUMENTS_BYTES {
        return rendered;
    }
    format!(
        "{}#fnv1a:{:016x}",
        &rendered[..floor_boundary(&rendered, MAX_ARGUMENTS_BYTES)],
        fnv1a64(rendered.as_bytes())
    )
}

/// Renders `value` deterministically: object keys sorted recursively,
/// arrays and scalars in `serde_json`'s own spelling. The point is that the
/// rendering depends only on the *value*, not on the order its object keys
/// happened to be built in — a guarantee `serde_json` itself does not make.
fn canonical_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(true) => out.push_str("true"),
        serde_json::Value::Bool(false) => out.push_str("false"),
        serde_json::Value::Number(number) => out.push_str(&number.to_string()),
        serde_json::Value::String(text) => {
            // `serde_json`'s escaping is injective (distinct strings render
            // distinctly) and deterministic; serializing a string cannot
            // fail, so this panic is unreachable.
            out.push_str(&serde_json::to_string(text).expect("string rendering is infallible"));
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("string rendering is infallible"));
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
    }
}

/// FNV-1a (64-bit, public-domain parameters): a dependency-free hash that
/// only has to be stable within one run, since the failure memory lives and
/// dies with the run.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
#[path = "failure_key_tests.rs"]
mod tests;
