//! System prompt assembly and per-turn context for agent turns — brief the
//! model on its context and memory, and carry the last-SQL hint on the user
//! turn (never the system prompt, where it would perturb the prefix cache).

use crate::connection::ConnectionRegistry;
use saya_agent::ContextBlock;
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

/// How the connected engines want an object named in SQL, plus the dialect
/// statement that names each engine and pins the SQL dialect.
///
/// Schema discovery reports every engine as catalog → schema → table, because
/// that is what a durable fact binds to. SQLite has no such depth in SQL: shown
/// `db.main.singer`, a model writes exactly that and the statement is rejected,
/// costing a round trip on nearly every question before it retries unqualified.
/// So each engine is told the fullest name it actually accepts — and no more.
///
/// The same section states the engine plainly and warns that the declared
/// column types may come from another engine. Benchmark evidence: one SQLite
/// database was a PostgreSQL dump, so its DDL still declared `jsonb`, `point`
/// and `timestamp with time zone`; schema discovery reported those declared
/// types, the model wrote Postgres syntax (`city->>'en'`, `coordinates[0]`),
/// and SQLite rejected every statement. The prompt must say the SQL dialect is
/// the connected engine's, whatever the DDL says — one or two sentences, on
/// every request. With several connections the engines are listed, so the
/// warning is worded per connected engine.
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
        [(engine, form)] => Some(format!(
            "The SQL dialect is {engine}'s, whatever the DDL says — the declared column \
             types in this database may come from another engine. Name objects as `{form}` \
             when writing SQL — the fullest form this engine accepts; an under-qualified \
             name cannot be recorded against a real object and an over-qualified one is a \
             syntax error."
        )),
        many => {
            let list = many
                .iter()
                .map(|(engine, form)| format!("- {engine}: `{form}`"))
                .collect::<Vec<_>>()
                .join("\n");
            Some(format!(
                "Name objects with the fullest form the target engine accepts, and no more \
                 — an under-qualified name cannot be recorded against a real object, and an \
                 over-qualified one is a syntax error:\n{list}\nThe SQL dialect is the \
                 connected engine's, whatever the DDL says — the declared column types in a \
                 database may come from another engine."
            ))
        }
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

/// The shape an answer must take. [`WORKING_GUIDANCE`] tells the model how to
/// proceed; this tells it how to present the result. Each clause fixes a
/// measured class of benchmark failure where the numbers were right and the
/// presentation was wrong: extra working columns, rounding the question never
/// asked for, non-ISO dates, a ranking where a single row was asked for, one
/// quantity answered where several were named, a measure word read loosely, a
/// metric qualifier applied to the whole population, a tie broken to fit a
/// limit, and a named period replaced by the rows that happened to appear.
/// Plain rules, no examples — this text rides on every request.
const ANSWER_CONTRACT: &str = "Answer the question exactly as asked:\n\
    - Return only the columns the question asks for; drop intermediate working columns.\n\
    - Do not round unless asked.\n\
    - Write dates as ISO YYYY-MM-DD.\n\
    - \"The highest\" or \"the top one\" means that single row, not the ranking it came from.\n\
    - Answer every quantity the question names; if it asks for two things, answer both.\n\
    - Read measure words literally: \"volume\" is units, \"revenue\" is money.\n\
    - A qualifier on a metric is not a qualifier on the population — filter the metric, not the rows.\n\
    - Keep every row tied at a cut-off; never drop a tie to fit a limit.\n\
    - When a period is named, enumerate that whole period, not only the rows that happen to appear in the data.";

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

/// Assembles the system prompt for a turn from connection registry context,
/// the memory briefing (if assisted and reachable), working guidance, the answer
/// contract, and the engine naming/dialect section.
///
/// This is **session-stable**: the same connections, memory mode, and reachability
/// produce a byte-identical system prompt across turns. The per-turn last-SQL
/// hint is deliberately absent — it rides the user turn as a context block (see
/// [`last_sql_hint_block`]) so it never perturbs the system block a provider's
/// prefix cache is keyed on.
pub(crate) fn assemble_system_prompt(
    registry: &ConnectionRegistry,
    memory_mode: MemoryMode,
    memory_reachable: bool,
) -> Option<String> {
    let base = registry.describe_context();
    let memory = if memory_reachable {
        memory_section(memory_mode)
    } else {
        None
    };

    let mut sections = Vec::new();
    if let Some(b) = base {
        sections.push(b);
    }
    if let Some(m) = memory {
        sections.push(m.to_string());
    }
    sections.push(WORKING_GUIDANCE.to_string());
    sections.push(ANSWER_CONTRACT.to_string());
    // Independent of memory mode: schema discovery hands the model a
    // catalog/schema/table tree for every engine, including the ones whose SQL
    // has no such depth, so without this the model writes back the shape it was
    // shown and the statement is rejected. This section also names the engine
    // and pins the dialect (declared types may come from another engine).
    if let Some(n) = naming_section(registry) {
        sections.push(n);
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
