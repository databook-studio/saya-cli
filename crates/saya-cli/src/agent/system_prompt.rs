//! System prompt assembly for agent turns — brief the model on its context and memory.

use crate::connection::ConnectionRegistry;
use saya_config::MemoryMode;

/// Memory briefing prompt included when assisted memory mode is active.
pub(crate) const MEMORY_SYSTEM_PROMPT: &str = "\
SAYA maintains durable knowledge about the user's databases across sessions. \
Confirmed facts relevant to the question are already supplied in context; there is no need to fetch them. \
The `contract_search` and `contract_read` tools are available for objects discovered mid-turn that were not in the supplied set. \
When the user states something durable about their data — what a word means, which column counts, what a table's grain is — restate it explicitly and precisely in the answer. What SAYA records is drawn from the turn, so a vague restatement is recorded vaguely.";

/// Memory section for the system prompt, present only in [`MemoryMode::Assisted`].
pub(crate) fn memory_section(mode: MemoryMode) -> Option<&'static str> {
    match mode {
        MemoryMode::Assisted => Some(MEMORY_SYSTEM_PROMPT),
        _ => None,
    }
}

/// Names the engine a single connection queries. `describe_context` stays
/// silent for one connection, so without this the model is never told it is
/// writing SQLite (or PostgreSQL, …) and pays a rejected query to find out.
/// With several connections the engines are already named by
/// `describe_context`, so this returns `None` to avoid restating them.
fn engine_section(registry: &ConnectionRegistry) -> Option<String> {
    let dialects: Vec<_> = registry.dialects().collect();
    match dialects.as_slice() {
        [dialect] => Some(format!("You are querying a {} database.", dialect.as_str())),
        _ => None,
    }
}

/// Coaching that applies to every turn regardless of how many databases are
/// connected: multi-step work is the norm, a failed query must not be repeated,
/// and a question no connected database can answer must end with a stated reason
/// rather than an endless loop. With no turn ceiling by default, the model
/// giving up well is the primary stopping condition.
const WORKING_GUIDANCE: &str = "Discover the schema before you query it; multi-step work is \
    expected. Do not repeat a query that already failed — change your approach instead. When the \
    question cannot be answered from this database, stop and say so, explaining what you tried and \
    what is missing: a missing table or column, data that is not present, or a question the schema \
    cannot express. Giving up with a reason is a correct outcome; looping is not.";

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

/// How the connected engines want an object named in SQL.
///
/// Schema discovery reports every engine as catalog → schema → table, because
/// that is what a durable fact binds to. SQLite has no such depth in SQL: shown
/// `db.main.singer`, a model writes exactly that and the statement is rejected,
/// costing a round trip on nearly every question before it retries unqualified.
/// So each engine is told the fullest name it actually accepts — and no more.
fn naming_section(registry: &ConnectionRegistry) -> Option<String> {
    let mut forms: Vec<(&str, &str)> = Vec::new();
    for dialect in registry.dialects() {
        let entry = (dialect.as_str(), dialect.qualified_name_form());
        if !forms.contains(&entry) {
            forms.push(entry);
        }
    }
    match forms.as_slice() {
        [] => None,
        [(_, form)] => Some(format!(
            "Name objects as `{form}` when writing SQL — the fullest form this engine \
             accepts. An under-qualified name cannot be recorded against a real object, \
             and an over-qualified one is a syntax error."
        )),
        many => {
            let list = many
                .iter()
                .map(|(engine, form)| format!("- {engine}: `{form}`"))
                .collect::<Vec<_>>()
                .join("\n");
            Some(format!(
                "Name objects with the fullest form the target engine accepts, and no \
                 more — an under-qualified name cannot be recorded against a real object, \
                 and an over-qualified one is a syntax error:\n{list}"
            ))
        }
    }
}

/// Hint for adapting the most recent executed SQL query.
fn last_sql_hint(sql: &str) -> String {
    format!(
        "For context, the most recent SQL you ran was:\n{sql}\n\nIf the user's request \
         refines, filters, sorts, or drills into that previous result, adapt this query \
         instead of rediscovering the schema from scratch."
    )
}

/// Assembles the complete system prompt from connection registry context,
/// memory briefing (if assisted), and optional last-SQL hint.
pub(crate) fn assemble_system_prompt(
    registry: &ConnectionRegistry,
    last_sql: Option<&str>,
    memory_mode: MemoryMode,
    memory_reachable: bool,
) -> Option<String> {
    let base = registry.describe_context();
    let memory = if memory_reachable {
        memory_section(memory_mode)
    } else {
        None
    };
    let hint = match last_sql {
        Some(sql) if !sql.trim().is_empty() => Some(last_sql_hint(sql)),
        _ => None,
    };

    let mut sections = Vec::new();
    if let Some(b) = base {
        sections.push(b);
    }
    if let Some(m) = memory {
        sections.push(m.to_string());
    }
    if let Some(engine) = engine_section(registry) {
        sections.push(engine);
    }
    sections.push(WORKING_GUIDANCE.to_string());
    // Independent of memory mode: schema discovery hands the model a
    // catalog/schema/table tree for every engine, including the ones whose SQL
    // has no such depth, so without this the model writes back the shape it was
    // shown and the statement is rejected.
    if let Some(n) = naming_section(registry) {
        sections.push(n);
    }
    if let Some(h) = hint {
        sections.push(h);
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

#[cfg(test)]
#[path = "system_prompt_tests.rs"]
mod tests;
