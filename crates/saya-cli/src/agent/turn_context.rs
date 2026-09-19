//! Per-turn context helpers for the system prompt: reachability and the
//! last-SQL user-turn hint.
//!
//! Split out of `system_prompt.rs` when that file approached the 250-line
//! hard cap. No behavior change: moved verbatim, re-exported through
//! `system_prompt` so existing call sites keep their paths.

use saya_agent::ContextBlock;

/// Whether this turn can honour what the memory section promises.
///
/// The section tells the model that confirmed facts are already in context and
/// that two named tools are available. Both are only true when the state store
/// opened *and* the privacy gate is open — with either shut, recall never ran
/// and the contract tools are not advertised. Briefing the model anyway would
/// have it look for supplied facts that are not there and call tools it does
/// not have, which is a worse failure than saying nothing.
pub(crate) fn memory_reachable(has_state_store: bool, allow_query_data: bool) -> bool {
    has_state_store && allow_query_data
}

/// Hint prose for adapting the most recent executed SQL query. Carried on the
/// user turn as a [`ContextBlock`] body (see [`last_sql_hint_block`]) — never in
/// the system prompt, where it would change on every follow-up that ran SQL and
/// forfeit the provider's prefix cache.
fn last_sql_hint(sql: &str) -> String {
    format!(
        "For context, the most recent SQL you ran was:\n{sql}\n\nIf the user's request \
         refines, filters, sorts, or drills into that previous result, adapt this query \
         instead of rediscovering the schema from scratch."
    )
}

/// Label for the last-SQL hint context block, which rides the user turn beside
/// the recall context block.
pub(crate) const LAST_SQL_BLOCK_LABEL: &str = "last-sql";

/// Builds the user-turn context block carrying the most recent SQL, so the
/// model can adapt it without the hint polluting the session-stable system
/// prompt. Returns `None` for empty/whitespace SQL. The body is untrusted data
/// rendered into the user turn by `saya_agent::build_messages` (quoted,
/// labelled, escaped) — never the system message.
pub(crate) fn last_sql_hint_block(sql: &str) -> Option<ContextBlock> {
    if sql.trim().is_empty() {
        return None;
    }
    Some(ContextBlock {
        label: LAST_SQL_BLOCK_LABEL.to_string(),
        body: last_sql_hint(sql),
        truncated: false,
    })
}
