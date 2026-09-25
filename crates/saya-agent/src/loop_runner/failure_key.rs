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
mod tests {
    use super::*;

    fn call_with(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments,
        }
    }

    /// Two calls whose argument objects were built with the same keys in
    /// different orders are the same call: the canonical rendering sorts
    /// object keys recursively, so the keys collide.
    #[test]
    fn arguments_built_in_different_key_orders_produce_one_key() {
        let first = call_with(
            "run_program",
            serde_json::json!({"command": "ls", "flags": {"all": true, "human": true}, "cwd": "/tmp"}),
        );
        let second = call_with(
            "run_program",
            serde_json::json!({"cwd": "/tmp", "flags": {"human": true, "all": true}, "command": "ls"}),
        );
        assert_eq!(key_of(&first), key_of(&second));
    }

    /// A changed argument value is a different call — canonicalisation of the
    /// serialization must not blur distinct content.
    #[test]
    fn a_changed_argument_value_is_a_different_key() {
        let first = call_with("run_program", serde_json::json!({"command": "ls"}));
        let second = call_with("run_program", serde_json::json!({"command": "ls -la"}));
        assert_ne!(key_of(&first), key_of(&second));
    }

    /// The tool name participates in the key: the same arguments under a
    /// different tool are a different invocation.
    #[test]
    fn the_tool_name_participates_in_the_key() {
        let first = call_with("run_program", serde_json::json!({"command": "ls"}));
        let second = call_with("other_tool", serde_json::json!({"command": "ls"}));
        assert_ne!(key_of(&first), key_of(&second));
    }

    /// The SQL path wins: a call carrying a `sql` string keys on the
    /// statement alone, so a different companion argument does not change the
    /// key (a known-bad statement must not look fresh on another connection).
    #[test]
    fn a_sql_carrying_call_keys_on_the_statement_alone() {
        let first = call_with(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT bad", "connection": "one"}),
        );
        let second = call_with(
            "bounded_sql_query",
            serde_json::json!({"connection": "two", "sql": "SELECT bad"}),
        );
        assert_eq!(key_of(&first), key_of(&second));
        assert_eq!(key_of(&first), CallKey::Sql("SELECT bad".into()));
    }

    /// A SQL statement and a tool call whose arguments spell alike do not
    /// collide — the variants keep the two paths apart by construction.
    #[test]
    fn a_sql_statement_and_a_tool_call_spelled_alike_do_not_collide() {
        assert_ne!(
            CallKey::Sql("SELECT 1".into()),
            CallKey::ToolCall("run_program".into(), "SELECT 1".into()),
        );
    }

    /// Huge arguments are bounded: identical huge calls still collide, the
    /// rendering never exceeds the prefix plus the hash suffix, and calls
    /// differing only past the prefix are told apart by the hash.
    #[test]
    fn a_huge_arguments_key_is_bounded_and_still_identifies_its_call() {
        let blob = "y".repeat(4_000);
        let first = key_of(&call_with(
            "run_program",
            serde_json::json!({"blob": blob.clone()}),
        ));
        let second = key_of(&call_with("run_program", serde_json::json!({"blob": blob})));
        assert_eq!(first, second, "identical huge arguments collide");
        let CallKey::ToolCall(_, ref rendering) = first else {
            panic!("a non-SQL call keys as a tool call");
        };
        // A prefix of at most MAX_ARGUMENTS_BYTES bytes, plus "#fnv1a:" and
        // 16 hex digits.
        assert!(
            rendering.len() <= MAX_ARGUMENTS_BYTES + 23,
            "the rendering is bounded, got {} bytes",
            rendering.len()
        );
        assert!(rendering.contains("#fnv1a:"));
        let truncated = key_of(&call_with(
            "run_program",
            serde_json::json!({"blob": format!("{}z", "y".repeat(4_000))}),
        ));
        assert_ne!(
            first, truncated,
            "a call differing past the prefix must not collide"
        );
    }

    /// The truncation prefix must never split a UTF-8 sequence: a rendering
    /// whose 512th byte falls inside a multi-byte character still produces a
    /// key (and the same arguments produce the same one).
    #[test]
    fn the_truncation_prefix_never_splits_a_code_point() {
        let blob = "€".repeat(1_000);
        let first = key_of(&call_with(
            "run_program",
            serde_json::json!({"blob": blob.clone()}),
        ));
        let second = key_of(&call_with("run_program", serde_json::json!({"blob": blob})));
        assert_eq!(first, second, "identical non-ASCII arguments collide");
    }
}
