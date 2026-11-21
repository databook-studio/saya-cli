//! The database part of the system prompt, built here and nowhere else.
//!
//! The memory briefing, the working guidance, the answer contract, the
//! engine-naming/dialect statement, and the session's database fact lines were
//! the chat prompt's (`agent/system_prompt.rs`, `agent/session_facts.rs`). They
//! live here so one copy of each text serves both callers: the plugin's
//! `system_section` states them on a chat run, and the old chat prompt renders
//! the same bytes from the same constants while it is still the path a chat
//! turn takes.

use saya_config::MemoryMode;
use saya_types::SqlDialect;

use crate::connection::ConnectionRegistry;

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

/// Coaching that applies to every turn regardless of how many databases are
/// connected: multi-step work is the norm, a failed attempt must not be
/// repeated, and a question nothing in the session can answer must end with a
/// stated reason rather than an endless loop. With no turn ceiling by default,
/// the model giving up well is the primary stopping condition. The database
/// specifics — schema discovery first, missing tables and columns — stay as
/// the database case of that rule.
pub(crate) const WORKING_GUIDANCE: &str = "Multi-step work is expected. Do not repeat an attempt that already \
    failed — change your approach instead. When working with a database, discover the schema before \
    you query it. If a material definition (grain, time column, metric, filter) is ambiguous and no \
    confirmed context resolves it, call request_clarification instead of assuming. When the question \
    cannot be answered from what is available in this session, stop \
    and say so, explaining what you tried and what is missing: a missing table or column, data that \
    is not present, or a question the schema cannot express. Giving up with a reason is a correct \
    outcome; looping is not.";

/// The shape an answer must take. [`WORKING_GUIDANCE`] tells the model how to
/// proceed; this tells it how to present the result. Each clause fixes an
/// observed class of failure where the numbers were right and the
/// presentation was wrong: extra working columns, rounding the question never
/// asked for, non-ISO dates, the operands of a computation handed back instead
/// of the computation the question named, a superlative answered with the
/// ranking it came from, one quantity answered where several were named, a
/// measure word read loosely, a metric qualifier applied to the whole
/// population, a tie broken to fit a limit, and a named period replaced by the
/// rows that happened to appear. Plain rules, no examples — this text rides on
/// every request.
pub(crate) const ANSWER_CONTRACT: &str = "Answer the question exactly as asked. These rules govern answers that report query results, and leave other answers untouched:\n\
    - Return only the columns the question asks for; drop intermediate working columns.\n\
    - Do not round unless asked.\n\
    - Write dates as ISO YYYY-MM-DD.\n\
    - If the question asks for a ratio, a percentage, or a difference, compute that value and answer with it — returning the operands alone stops one step short.\n\
    - A superlative — \"the fastest\", \"the highest\", \"the top one\", \"the fewest\" — asks which one: answer with that row and the value that makes it so, not the ranking it came from. Every row tied with it is part of the answer.\n\
    - Answer every quantity the question names; if it asks for two things, answer both.\n\
    - Read measure words literally: \"volume\" is units, \"revenue\" is money.\n\
    - A qualifier on a metric is not a qualifier on the population — filter the metric, not the rows.\n\
    - Keep every row tied at a cut-off; never drop a tie to fit a limit.\n\
    - When a period is named, enumerate that whole period, not only the rows that happen to appear in the data.";

/// The connections a database prompt is rendered from, owned by the caller.
///
/// The plugin holds one across a run, so the section it states is composed from
/// the connection set the run resolved — never re-read per turn, where a change
/// would move the request prefix a provider's prompt cache is keyed on.
pub(crate) struct DatabaseFacts {
    connections: Vec<(String, SqlDialect)>,
}

impl DatabaseFacts {
    /// The facts of the run's connection set, in session order.
    pub(crate) fn of(registry: &ConnectionRegistry) -> Self {
        Self {
            connections: registry
                .entries()
                .into_iter()
                .map(|(name, entry)| (name.to_owned(), entry.dialect))
                .collect(),
        }
    }

    /// The session's database fact lines, one per connection: what the session
    /// is connected to, in the order the session connected.
    pub(crate) fn session_lines(&self) -> Vec<String> {
        self.connections
            .iter()
            .map(|(name, dialect)| format!("- Database `{name}` ({}).", dialect.as_str()))
            .collect()
    }

    /// How the connected engines want an object named in SQL, plus the dialect
    /// statement that names each engine and pins the SQL dialect.
    ///
    /// Schema discovery reports every engine as catalog → schema → table,
    /// because that is what a durable fact binds to. SQLite has no such depth in
    /// SQL: shown `db.main.singer`, a model writes exactly that and the
    /// statement is rejected, costing a round trip on nearly every question
    /// before it retries unqualified. So each engine is told the fullest name it
    /// actually accepts — and no more.
    ///
    /// The same section states the engine plainly and warns that the declared
    /// column types may come from another engine. Observed evidence: one SQLite
    /// database was a PostgreSQL dump, so its DDL still declared `jsonb`,
    /// `point` and `timestamp with time zone`; schema discovery reported those
    /// declared types, the model wrote Postgres syntax (`city->>'en'`,
    /// `coordinates[0]`), and SQLite rejected every statement. The prompt must
    /// say the SQL dialect is the connected engine's, whatever the DDL says —
    /// one or two sentences, on every request. With several connections the
    /// engines are listed, so the warning is worded per connected engine.
    pub(crate) fn naming_section(&self) -> Option<String> {
        let mut forms: Vec<(&str, &str)> = Vec::new();
        for (_, dialect) in &self.connections {
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
}

/// What a chat run hands the plugin once, at composition: the connection facts
/// and the memory state the run resolved.
pub(crate) struct ChatPrompt {
    facts: DatabaseFacts,
    memory: MemoryMode,
    memory_reachable: bool,
}

impl ChatPrompt {
    /// The prompt inputs a chat run resolved from its connection set.
    pub(crate) fn of(
        registry: &ConnectionRegistry,
        memory: MemoryMode,
        memory_reachable: bool,
    ) -> Self {
        Self {
            facts: DatabaseFacts::of(registry),
            memory,
            memory_reachable,
        }
    }

    /// The chat run's database part of the system prompt: the same sections, in
    /// the same order, that the chat prompt assembles today — the memory
    /// briefing (only when the run can honour it), the session's database fact
    /// lines, the working guidance, the answer contract, and the engine naming
    /// statement.
    pub(crate) fn section(&self) -> String {
        let mut sections: Vec<String> = Vec::new();
        if self.memory_reachable
            && let Some(memory) = memory_section(self.memory)
        {
            sections.push(memory.to_string());
        }
        let lines = self.facts.session_lines();
        if !lines.is_empty() {
            sections.push(lines.join("\n"));
        }
        sections.push(WORKING_GUIDANCE.to_string());
        sections.push(ANSWER_CONTRACT.to_string());
        if let Some(naming) = self.facts.naming_section() {
            sections.push(naming);
        }
        sections.join("\n\n")
    }
}
