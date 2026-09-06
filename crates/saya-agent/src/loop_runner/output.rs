use crate::{AgentEvent, ProviderError, ToolMetadata};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct AgentLimits {
    /// Ceiling on the number of provider turns, or `None` to run without a
    /// turn ceiling. `None` is the default: the loop stops when the model
    /// stops calling tools, when it is cancelled, or — when a ceiling is set
    /// — by salvaging the best answer from the work already done. There is no
    /// upper limit on a set value.
    pub max_turns: Option<usize>,
    /// Ceiling on the total number of tool calls across the whole run, or
    /// `None` for no ceiling. Same stopping policy as [`Self::max_turns`].
    pub max_tool_calls: Option<usize>,
    /// Whether the loop may execute tools that declare
    /// [`LocalStateEffect::WriteCandidate`](crate::LocalStateEffect::WriteCandidate).
    /// Defaults to **not permitted**: a tool that can write a candidate claim
    /// must not start writing merely because it was registered.
    pub permit_candidate_writes: bool,
    /// Ceiling on the approximate byte size of the conversation the loop has
    /// assembled (assistant turns plus tool results grow it past the pre-loop
    /// history budget). Breaching it trims the oldest tool-result groups
    /// rather than sending an ever-growing payload to the provider.
    pub context_byte_budget: usize,
}
impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_turns: None,
            max_tool_calls: None,
            permit_candidate_writes: false,
            context_byte_budget: 256 * 1024,
        }
    }
}

/// Environment variable naming the turn ceiling (`SAYA_AGENT_MAX_TURNS`).
pub const MAX_TURNS_ENV: &str = "SAYA_AGENT_MAX_TURNS";
/// Environment variable naming the tool-call ceiling (`SAYA_AGENT_MAX_TOOL_CALLS`).
pub const MAX_TOOL_CALLS_ENV: &str = "SAYA_AGENT_MAX_TOOL_CALLS";

/// The tool name the model calls to designate the SQL that answers the
/// question. Recognised by the loop at the terminal turn; the SQL text is
/// carried on [`AgentOutput::answer_sql`] and an `AnswerDesignated` event, not
/// executed as a tool.
pub const DESIGNATE_ANSWER_TOOL: &str = "designate_answer";

/// Reads the turn and tool-call ceilings from the environment through `get`.
///
/// Each is optional: `None` (unset or unparseable) leaves the loop unbounded
/// for that ceiling, and a set value imposes no upper limit. `get` is a
/// callback rather than a direct `std::env::var` so the parsing is testable
/// without mutating process-global environment; the caller supplies the env
/// source.
pub fn budgets_from_env(get: impl Fn(&str) -> Option<String>) -> (Option<usize>, Option<usize>) {
    (
        parse_budget(get(MAX_TURNS_ENV)),
        parse_budget(get(MAX_TOOL_CALLS_ENV)),
    )
}

fn parse_budget(value: Option<String>) -> Option<usize> {
    value.and_then(|raw| raw.trim().parse().ok())
}
// `events` holds `AgentEvent`, which carries a `serde_json::Value` and is
// therefore `PartialEq` but not `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentOutput {
    pub answer: String,
    pub events: Vec<AgentEvent>,
    pub used_bounded_sql_query: bool,
    pub tool_metadata: Vec<ToolMetadata>,
    /// Token counts summed over every provider turn of this run (zero when
    /// the provider does not report usage).
    pub usage: crate::TokenUsage,
    /// Token counts the post-turn extraction call reported, when it produced a
    /// response. `None` covers both "no extraction ran" (learning disabled or
    /// the gate declined) and "extraction ran but produced no response"
    /// (provider error or timeout) — absent is not zero, so the recorder must
    /// not fold it in as a row of zeros. Kept separate from `usage` so the
    /// answering total's meaning is unchanged and `/usage` can label the two
    /// calls apart.
    pub learning_usage: Option<crate::TokenUsage>,
    /// `true` when the answer is the best the model could produce after a
    /// budget ran out mid-run, rather than a natural completion. The run did
    /// not finish on its own; the caller may surface this so a reader knows the
    /// answer is bounded-effort, not a clean result.
    pub truncated: bool,
    /// The SQL the model designated as the query that answers the question, or
    /// `None` when the model did not designate one. Optional by design: a turn
    /// that never designates works exactly as before. Carries the SQL text
    /// only — already user-visible via tool-call detail — never result rows.
    pub answer_sql: Option<String>,
}
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("{0}")]
    Provider(#[from] ProviderError),
    #[error("agent limit reached: {0}")]
    Limit(&'static str),
    #[error("provider returned an unsupported tool call")]
    InvalidToolCall,
    #[error("conversation history is invalid")]
    InvalidHistory,
    #[error("request cancelled")]
    Cancelled,
}

/// Approximate serialized size of a message, including tool-call arguments,
/// which dominate real growth during multi-turn runs.
pub(super) fn message_size(message: &crate::ChatMessage) -> usize {
    message.content.len()
        + message.role.len()
        + message
            .tool_calls
            .iter()
            .map(|call| {
                call.id.len()
                    + call.name.len()
                    + serde_json::to_string(&call.arguments).map_or(0, |text| text.len())
            })
            .sum::<usize>()
}

/// A completion summary for a tool result, flagged when the result was
/// truncated to fit the conversation budget so the model is told it saw a cut
/// result rather than a complete one — the model must not be misled.
pub(super) fn completion_summary(base: &str, truncated: bool) -> String {
    if truncated {
        format!("{base} (truncated)")
    } else {
        base.to_owned()
    }
}

/// The marker appended to an in-place-truncated tool message so the model
/// knows the result it received was cut.
const TRUNCATED_MARKER: &str = "…[truncated: tool result exceeded the conversation byte budget]";

/// Enforces the intra-loop conversation byte budget in place, never aborting
/// the run. Under the budget the conversation is left untouched. Over it, a
/// single compaction evicts the oldest tool-result groups and replaces them
/// with one short digest (the statements that ran and the shapes of their
/// results — never row values), keeping the system message and the most
/// recent groups intact. One compaction rewrites the prefix once, so a prompt
/// cache misses once per compaction rather than once per turn; the loop then
/// appends again until the budget is exceeded anew.
///
/// An already-sent tool message is never rewritten: only the newest group —
/// the one part of the conversation not yet sent to the provider at the point
/// of trimming — may be truncated, and only when it alone (with the digest)
/// still exceeds the budget.
pub(super) fn trim_to_budget(messages: &mut Vec<crate::ChatMessage>, budget: usize) {
    let total = messages.iter().map(message_size).sum::<usize>();
    if total <= budget {
        return;
    }
    compact(messages, budget);
}

/// One compaction: evict the oldest tool-result groups (and any prior digest),
/// replacing them with a single digest, keeping the most recent groups that
/// still fit. If keeping only the newest group with the digest still does not
/// fit, truncate the newest group's largest `tool` result (the one part not
/// yet sent) rather than rewrite a seen message.
fn compact(messages: &mut Vec<crate::ChatMessage>, budget: usize) {
    let groups = tool_groups(messages);
    let n = groups.len();
    if n <= 1 {
        // No older group to evict: the newest group alone is over budget (plus
        // any prior digest). Truncate its `tool` result; do not insert a digest
        // for nothing.
        truncate_newest_group(messages, budget);
        return;
    }
    let Some(region_start) = first_evictable_index(messages, &groups) else {
        return;
    };
    // Keep the most recent groups that fit with a single digest replacing the
    // evicted prefix. Try the largest keep count first; evict at least one.
    for keep in (1..n).rev() {
        let kept_start = groups[n - keep].start;
        let digest = super::compaction::build_digest(&messages[region_start..kept_start]);
        let head: usize = messages[..region_start].iter().map(message_size).sum();
        let kept: usize = messages[kept_start..].iter().map(message_size).sum();
        if head + message_size(&digest) + kept <= budget {
            replace_with_digest(messages, region_start, kept_start, digest);
            return;
        }
    }
    // Evicting all but the newest group still does not fit: evict every group
    // except the newest, insert the digest, then truncate the newest group's
    // largest `tool` result to fit.
    let kept_start = groups[n - 1].start;
    let digest = super::compaction::build_digest(&messages[region_start..kept_start]);
    replace_with_digest(messages, region_start, kept_start, digest);
    truncate_newest_group(messages, budget);
}

/// Drains `messages[start..end)` and inserts `digest` at `start` in its place.
fn replace_with_digest(
    messages: &mut Vec<crate::ChatMessage>,
    start: usize,
    end: usize,
    digest: crate::ChatMessage,
) {
    messages.drain(start..end);
    messages.insert(start, digest);
}

/// The index where the evictable region begins: the first tool group, or a
/// prior digest sitting just before it — whichever is earlier. Everything
/// before it (system, user, history) is the head and is never evicted.
fn first_evictable_index(
    messages: &[crate::ChatMessage],
    groups: &[std::ops::Range<usize>],
) -> Option<usize> {
    let first_group = groups.first().map(|range| range.start);
    let first_digest = messages.iter().position(super::compaction::is_digest);
    match (first_group, first_digest) {
        (Some(g), Some(d)) => Some(g.min(d)),
        (Some(g), None) => Some(g),
        (None, Some(d)) => Some(d),
        (None, None) => None,
    }
}

/// Truncates the newest group's largest `tool` message in place so the whole
/// conversation fits `budget`. The newest group is the one part not yet sent
/// to the provider, so rewriting it does not mislead the model about text it
/// already saw. Stops when the budget is met or no truncation can make
/// progress, never touching any other `tool` message.
fn truncate_newest_group(messages: &mut [crate::ChatMessage], budget: usize) {
    loop {
        let total = messages.iter().map(message_size).sum::<usize>();
        if total <= budget {
            return;
        }
        let Some(newest) = tool_groups(messages).last().cloned() else {
            return;
        };
        let Some(idx) = (newest.start + 1..newest.end)
            .filter(|&i| messages[i].role == "tool")
            .max_by_key(|&i| messages[i].content.len())
        else {
            return;
        };
        let content_len = messages[idx].content.len();
        let overhead = total - content_len;
        let target = budget.saturating_sub(overhead + TRUNCATED_MARKER.len());
        if target >= content_len {
            return;
        }
        let bound = super::tools::floor_boundary(&messages[idx].content, target);
        if bound + TRUNCATED_MARKER.len() >= content_len {
            return;
        }
        messages[idx].content.truncate(bound);
        messages[idx].content.push_str(TRUNCATED_MARKER);
    }
}

/// Returns the index ranges of complete tool-result groups in `messages`: each
/// is an assistant turn carrying tool calls followed by its consecutive `tool`
/// messages. Ranges are half-open `[start, end)`, suitable for `Vec::drain`.
fn tool_groups(messages: &[crate::ChatMessage]) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        if !messages[i].tool_calls.is_empty() {
            let start = i;
            i += 1;
            while i < messages.len() && messages[i].role == "tool" {
                i += 1;
            }
            groups.push(start..i);
        } else {
            i += 1;
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_are_unbounded() {
        let limits = AgentLimits::default();
        assert!(limits.max_turns.is_none(), "no turn ceiling by default");
        assert!(
            limits.max_tool_calls.is_none(),
            "no tool-call ceiling by default"
        );
    }

    #[test]
    fn budgets_from_env_sets_each_ceiling_independently_with_no_upper_limit() {
        let lookup = |name: &str| match name {
            "SAYA_AGENT_MAX_TURNS" => Some("1000000".to_string()),
            "SAYA_AGENT_MAX_TOOL_CALLS" => Some("7".to_string()),
            _ => None,
        };
        let (turns, tool_calls) = budgets_from_env(lookup);
        assert_eq!(turns, Some(1_000_000));
        assert_eq!(tool_calls, Some(7));
    }

    #[test]
    fn budgets_from_env_is_unbounded_when_unset_or_unparseable() {
        let (turns, tool_calls) = budgets_from_env(|_| None);
        assert!(turns.is_none() && tool_calls.is_none());
        // A set but unparseable value is treated as unset, not as zero: a typo
        // cannot silently collapse the ceiling to the smallest bound.
        let lookup = |name: &str| match name {
            "SAYA_AGENT_MAX_TURNS" => Some("not-a-number".to_string()),
            _ => None,
        };
        let (turns, _) = budgets_from_env(lookup);
        assert!(turns.is_none());
    }

    /// An already-sent tool message — a result the model has already seen in an
    /// earlier turn — must never be rewritten by the budget trim. Rewriting it
    /// would feed the model different text than it saw before, worse than
    /// dropping the oldest context. Only the newest group (the turn not yet
    /// sent at the point of trimming) may be truncated.
    ///
    /// The conversation here carries a large, already-sent tool result that is
    /// the largest `tool` message, alongside the current turn's group with a
    /// small result. The budget is exceeded. The trim must leave the sent
    /// result byte-identical and still bring the conversation under the budget
    /// by truncating the newest (unsent) result.
    #[test]
    fn trim_never_mutates_an_already_sent_tool_message() {
        let old_sent = "X".repeat(200);
        let new_result = "Y".repeat(120);
        let mut messages = vec![
            crate::ChatMessage::text("system", "s"),
            crate::ChatMessage::text("user", "ask"),
            // An already-sent tool result from an earlier turn. It has no
            // preceding assistant tool-call turn in this slice, so it is not
            // part of the newest group — it is exactly the kind of message the
            // trim must not touch.
            crate::ChatMessage {
                role: "tool".into(),
                content: old_sent.clone(),
                tool_calls: Vec::new(),
                tool_call_id: Some("c-old".into()),
            },
            // The newest group — the current turn's call, not yet sent.
            crate::ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![crate::ToolCall {
                    id: "c1".into(),
                    name: "t".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            },
            crate::ChatMessage {
                role: "tool".into(),
                content: new_result.clone(),
                tool_calls: Vec::new(),
                tool_call_id: Some("c1".into()),
            },
        ];
        let total: usize = messages.iter().map(message_size).sum();
        let newest_tool = message_size(&messages[4]);
        let everything_else = total - newest_tool;
        // Budget: over the total so the trim acts, but with enough headroom that
        // truncating only the newest (unsent) result can bring it back under.
        let budget = everything_else + TRUNCATED_MARKER.len() + 10;
        assert!(
            budget < total,
            "test setup must exceed the budget: {budget} vs {total}"
        );
        trim_to_budget(&mut messages, budget);
        assert_eq!(
            messages[2].content, old_sent,
            "an already-sent tool message must not be mutated"
        );
        let after: usize = messages.iter().map(message_size).sum();
        assert!(
            after <= budget,
            "the budget must still bind after trim: {after} vs {budget}"
        );
    }

    /// A one-message tool-result group: the assistant turn that issued the call
    /// plus its `tool` result, built from a SQL statement and a result value.
    fn group(call_id: &str, sql: &str, result: serde_json::Value) -> Vec<crate::ChatMessage> {
        vec![
            crate::ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![crate::ToolCall {
                    id: call_id.into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql": sql}),
                }],
                tool_call_id: None,
            },
            crate::ChatMessage {
                role: "tool".into(),
                content: serde_json::to_string(&result).expect("result serializes"),
                tool_calls: Vec::new(),
                tool_call_id: Some(call_id.into()),
            },
        ]
    }

    /// Under the budget, the conversation is left untouched — byte-identical,
    /// not silently reorganized. (Part 3: compaction only fires when over.)
    #[test]
    fn trim_under_budget_leaves_the_conversation_untouched() {
        let mut messages = vec![
            crate::ChatMessage::text("system", "sys"),
            crate::ChatMessage::text("user", "ask"),
        ];
        messages.extend(group(
            "c0",
            "SELECT 1",
            serde_json::json!({"columns":["a"],"rows":[[1]],"row_count":1,"truncated":false,"executed_sql":"SELECT 1"}),
        ));
        let snapshot = messages.clone();
        let total: usize = messages.iter().map(message_size).sum();
        trim_to_budget(&mut messages, total + 1024);
        assert_eq!(
            messages, snapshot,
            "under-budget trim must not touch the conversation"
        );
    }

    /// Over the budget, one compaction evicts the oldest tool-result groups and
    /// replaces them with a digest, keeping the system message and the newest
    /// group byte-identical. The newest group is the freshest context; the
    /// system message carries policy. Neither is rewritten.
    #[test]
    fn compaction_keeps_the_system_message_and_newest_group_byte_identical() {
        let big = serde_json::json!({"columns":["a"],"rows":[["B".repeat(4000)]],"row_count":1,"truncated":false,"executed_sql":"SELECT a FROM x"});
        let small = serde_json::json!({"columns":["c"],"rows":[["ok"]],"row_count":1,"truncated":false,"executed_sql":"SELECT c FROM z"});
        let mut messages = vec![
            crate::ChatMessage::text("system", "sys"),
            crate::ChatMessage::text("user", "ask"),
        ];
        messages.extend(group("c0", "SELECT a FROM x", big.clone()));
        messages.extend(group("c1", "SELECT b FROM y", big));
        messages.extend(group("c2", "SELECT c FROM z", small));
        let system_snapshot = messages[0].clone();
        let newest_snapshot: Vec<crate::ChatMessage> = messages[messages.len() - 2..].to_vec();
        // Budget large enough that evicting the two big groups with a digest
        // fits while keeping the newest group intact, but too small to keep any
        // of the big groups (each dwarfs the allowance).
        let head: usize = messages[..2].iter().map(message_size).sum();
        let newest_size: usize = messages[messages.len() - 2..]
            .iter()
            .map(message_size)
            .sum();
        let budget = head + newest_size + 1024;
        trim_to_budget(&mut messages, budget);
        assert_eq!(
            messages[0], system_snapshot,
            "the system message must be byte-identical after compaction"
        );
        let tail: Vec<crate::ChatMessage> = messages[messages.len() - 2..].to_vec();
        assert_eq!(
            tail, newest_snapshot,
            "the newest group must be byte-identical after compaction"
        );
        let after: usize = messages.iter().map(message_size).sum();
        assert!(after <= budget, "the budget must bind: {after} vs {budget}");
    }

    /// The compaction digest names the statements it replaced and carries no
    /// row value — only the statement text and the result's shape (row count
    /// and column names). A planted cell value in an evicted result must not
    /// survive into the digest.
    #[test]
    fn compaction_digest_names_statements_and_carries_no_row_value() {
        let planted = "PLANTED_CELL_SENTINEL_42";
        // The evicted result is large (a big cell value) so a budget that
        // keeps the newest group intact genuinely exceeds the total and fires
        // a compaction. The planted sentinel lives in a row VALUE.
        let evicted_result = serde_json::json!({
            "columns": ["id", "secret"],
            "rows": [[1, format!("{planted}{}", "P".repeat(3000))]],
            "row_count": 1,
            "truncated": false,
            "executed_sql": "SELECT id, secret FROM t"
        });
        let newest_result = serde_json::json!({
            "columns": ["one"], "rows": [[1]], "row_count": 1, "truncated": false, "executed_sql": "SELECT 1"
        });
        let mut messages = vec![
            crate::ChatMessage::text("system", "sys"),
            crate::ChatMessage::text("user", "ask"),
        ];
        messages.extend(group("c0", "SELECT id, secret FROM t", evicted_result));
        messages.extend(group("c1", "SELECT 1", newest_result));
        let head: usize = messages[..2].iter().map(message_size).sum();
        let newest_size: usize = messages[messages.len() - 2..]
            .iter()
            .map(message_size)
            .sum();
        // Budget keeps the newest group intact (head + newest + allowance) but
        // is far below the total, so the large evicted group is compacted.
        let budget = head + newest_size + 256;
        let total: usize = messages.iter().map(message_size).sum();
        assert!(
            budget < total,
            "test setup must exceed the budget: {budget} vs {total}"
        );
        trim_to_budget(&mut messages, budget);
        let digest = messages
            .iter()
            .find(|message| crate::loop_runner::compaction::is_digest(message))
            .expect("a digest must replace the evicted group");
        assert!(
            digest.content.contains("SELECT id, secret FROM t"),
            "the digest must name the evicted statement: {}",
            digest.content
        );
        assert!(
            !digest.content.contains(planted),
            "a row value must never appear in the digest: {}",
            digest.content
        );
        let after: usize = messages.iter().map(message_size).sum();
        assert!(after <= budget, "the budget must bind: {after} vs {budget}");
    }
}
