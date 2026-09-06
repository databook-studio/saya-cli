use super::*;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use async_trait::async_trait;
use saya_agent::{build_messages, turn_bytes};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

struct DummyConnector {
    dialect: SqlDialect,
}

#[async_trait]
impl saya_connectors::DatabaseConnector for DummyConnector {
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

fn single_registry(name: &str) -> ConnectionRegistry {
    let mut reg = ConnectionRegistry::new(name);
    reg.insert(
        name,
        ConnectionEntry {
            connector: Box::new(DummyConnector {
                dialect: SqlDialect::Postgres,
            }),
            dialect: SqlDialect::Postgres,
            profile_id: None,
        },
    );
    reg
}

fn multi_registry() -> ConnectionRegistry {
    let mut reg = ConnectionRegistry::new("db1");
    reg.insert(
        "db1",
        ConnectionEntry {
            connector: Box::new(DummyConnector {
                dialect: SqlDialect::Postgres,
            }),
            dialect: SqlDialect::Postgres,
            profile_id: None,
        },
    );
    reg.insert(
        "db2",
        ConnectionEntry {
            connector: Box::new(DummyConnector {
                dialect: SqlDialect::Mysql,
            }),
            dialect: SqlDialect::Mysql,
            profile_id: None,
        },
    );
    reg
}

#[test]
fn memory_section_appears_under_assisted_and_absent_under_off() {
    assert!(memory_section(MemoryMode::Assisted).is_some());
    assert!(memory_section(MemoryMode::Off).is_none());

    let text = memory_section(MemoryMode::Assisted).unwrap();
    // Briefing contents verification
    assert!(text.contains("durable knowledge"));
    assert!(text.contains("Confirmed facts relevant to the question are already supplied"));
    assert!(text.contains("contract_search"));
    assert!(text.contains("contract_read"));
    assert!(text.contains("restate it explicitly and precisely in the answer"));
    // The naming rule is not here: its correct wording depends on the engine,
    // so it is built per connection alongside this section.
    assert!(!text.contains("catalog.schema.object"));
}

#[test]
fn assemble_system_prompt_single_connection_off() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, MemoryMode::Off, true).expect("a prompt");
    // With one connection and memory off there is no context and no briefing —
    // but the model is still shown a catalog/schema/table tree by schema
    // discovery, so it still has to be told what SQL will accept.
    assert!(!prompt.contains("durable knowledge"));
    assert!(prompt.contains("catalog.schema.object"));
}

#[test]
fn assemble_system_prompt_single_connection_assisted() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, MemoryMode::Assisted, true).expect("a prompt");
    assert!(prompt.starts_with(MEMORY_SYSTEM_PROMPT));
    // A single PostgreSQL connection: the memory briefing, then its naming rule.
    assert!(prompt.contains("catalog.schema.object"));
}

/// The previous turn's SQL no longer reaches the system prompt; it rides the
/// user turn as a context block (see [`last_sql_hint_reaches_user_turn`]). With
/// memory off, the system prompt carries no hint and no SQL text.
#[test]
fn assemble_system_prompt_with_last_sql_and_memory_off() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, MemoryMode::Off, true).expect("a prompt");
    assert!(!prompt.contains("durable knowledge"));
    assert!(
        !prompt.contains("most recent SQL you ran was"),
        "the hint must not appear in the system prompt: {prompt}"
    );
}

/// The hint is absent from the system prompt under assisted memory too — the
/// hint block is built separately and attached to the user turn.
#[test]
fn assemble_system_prompt_with_last_sql_and_assisted() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, MemoryMode::Assisted, true).expect("a prompt");
    assert!(prompt.contains(MEMORY_SYSTEM_PROMPT));
    assert!(
        !prompt.contains("most recent SQL you ran was"),
        "the hint must not appear in the system prompt: {prompt}"
    );
}

/// The multi-connection system prompt keeps its section order (connections,
/// memory, guidance) and never carries the previous SQL.
#[test]
fn assemble_system_prompt_multi_connection_assisted_and_sql() {
    let reg = multi_registry();
    let prompt = assemble_system_prompt(&reg, MemoryMode::Assisted, true).expect("a prompt");

    // Contains all three sections in expected order.
    let conn_idx = prompt.find("Available database connections").unwrap();
    let mem_idx = prompt.find("SAYA maintains durable knowledge").unwrap();
    let guidance_idx = prompt.find("Discover the schema").unwrap();

    assert!(conn_idx < mem_idx);
    assert!(mem_idx < guidance_idx);
    assert!(
        !prompt.contains("most recent SQL you ran was"),
        "the hint must not appear in the system prompt: {prompt}"
    );
    // The dialect statement rides every request, multi-connection included: the
    // engines are listed, then the dialect is pinned whatever the DDL declares.
    assert!(
        prompt.contains("The SQL dialect is the connected engine's, whatever the DDL says"),
        "the dialect statement must appear on every request: {prompt}"
    );
    assert!(
        prompt.contains("- postgresql: `catalog.schema.object`"),
        "the multi-connection list must name each engine: {prompt}"
    );
}

#[test]
fn assemble_system_prompt_stays_within_budget() {
    let reg = multi_registry();
    let prompt = assemble_system_prompt(&reg, MemoryMode::Assisted, true);
    assert!(prompt.is_some());
    let system_text = prompt.unwrap();

    // Check raw length is tiny compared to a conversation budget.
    assert!(system_text.len() < 4000);

    // The system prompt plus an ordinary question is far below any realistic
    // context_byte_budget — the loop bounds the conversation, not a start-of-run
    // cap, so this is a sanity check, not an enforced ceiling.
    let bytes = turn_bytes(Some(&system_text), &[], "How many orders were placed?");
    assert!(bytes < 32 * 1024);
}

/// Assisted, but the state store did not open or the privacy gate is shut: the
/// section promises supplied facts and two tools that this turn does not have,
/// so it must not appear.
#[test]
fn memory_section_is_absent_when_memory_is_configured_on_but_unreachable() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, MemoryMode::Assisted, false).expect("a prompt");
    assert!(
        !prompt.contains("durable knowledge"),
        "memory is unreachable, so the briefing must not claim otherwise: {prompt}"
    );

    assert!(memory_reachable(true, true));
    assert!(
        !memory_reachable(false, true),
        "no state store, no briefing"
    );
    assert!(
        !memory_reachable(true, false),
        "privacy gate shut, no briefing"
    );
}

fn registry_with(dialect: SqlDialect) -> ConnectionRegistry {
    let mut reg = ConnectionRegistry::new("db");
    reg.insert(
        "db",
        ConnectionEntry {
            connector: Box::new(DummyConnector { dialect }),
            dialect,
            profile_id: None,
        },
    );
    reg
}

/// SQLite has no catalog and no schema, so a three-part name is a syntax error
/// there. Telling the model to write one regardless costs a rejected query and
/// a wasted round trip on nearly every question before it corrects itself.
#[test]
fn the_naming_rule_matches_what_the_engine_accepts() {
    let sqlite = assemble_system_prompt(
        &registry_with(SqlDialect::Sqlite),
        MemoryMode::Assisted,
        true,
    )
    .expect("a prompt");
    assert!(
        !sqlite.contains("catalog.schema.object"),
        "SQLite cannot parse a three-part name, so the prompt must not ask for one: {sqlite}"
    );

    let postgres = assemble_system_prompt(
        &registry_with(SqlDialect::Postgres),
        MemoryMode::Assisted,
        true,
    )
    .expect("a prompt");
    assert!(
        postgres.contains("catalog.schema.object"),
        "PostgreSQL does accept the three-part name and should still be asked for it: {postgres}"
    );
}

/// The rule exists so a remembered fact binds to a real object, and that need
/// does not go away on an engine with fewer name parts — the prompt must still
/// ask for the fullest name the engine has.
#[test]
fn every_engine_is_still_told_to_qualify_names() {
    for dialect in [
        SqlDialect::Postgres,
        SqlDialect::Mysql,
        SqlDialect::Sqlite,
        SqlDialect::DuckDb,
        SqlDialect::Snowflake,
    ] {
        let prompt = assemble_system_prompt(&registry_with(dialect), MemoryMode::Assisted, true)
            .expect("a prompt");
        assert!(
            prompt.contains(dialect.qualified_name_form()),
            "{} must be told its own name form: {prompt}",
            dialect.as_str()
        );
    }
}

/// Writing a name the engine can parse is not a memory concern: schema
/// discovery shows the same catalog/schema/table tree whether memory is on or
/// off, so the rule that keeps SQL valid has to be present either way.
#[test]
fn the_naming_rule_is_present_with_memory_off() {
    let prompt = assemble_system_prompt(&registry_with(SqlDialect::Sqlite), MemoryMode::Off, false)
        .expect("a prompt");
    assert!(
        prompt.contains(SqlDialect::Sqlite.qualified_name_form()),
        "the SQL naming rule must survive memory being off: {prompt}"
    );
    assert!(
        !prompt.contains("durable knowledge"),
        "memory is off, so the memory briefing must stay absent: {prompt}"
    );
}

/// A single connection is never named by `describe_context` (it stays silent
/// for one connection), so the prompt itself must name the engine — the model
/// is otherwise never told it is writing SQLite. The prompt must also coach
/// multi-step work, refusing to repeat a failed query, and giving up with a
/// reason when the question cannot be answered from this database.
#[test]
fn single_connection_prompt_names_engine_and_guides_giving_up() {
    let prompt =
        assemble_system_prompt(&registry_with(SqlDialect::Postgres), MemoryMode::Off, false)
            .expect("a prompt");
    assert!(
        prompt.contains("postgresql"),
        "the engine must be named for a single connection: {prompt}"
    );
    assert!(
        prompt.contains("multi-step"),
        "the prompt must say multi-step work is expected: {prompt}"
    );
    assert!(
        prompt.contains("Do not repeat a query that already failed"),
        "the prompt must tell the model not to repeat a failed query: {prompt}"
    );
    assert!(
        prompt.contains("Giving up with a reason"),
        "the prompt must sanction giving up with a reason: {prompt}"
    );
    assert!(
        prompt.contains("stop and say so"),
        "the prompt must tell the model to stop and explain when it cannot answer: {prompt}"
    );
}

/// The give-up guidance is not a single-connection concern: with several
/// databases the model can still hit a question no connected database can
/// answer, so the coaching must be present there too.
#[test]
fn multi_connection_prompt_also_guides_giving_up() {
    let prompt =
        assemble_system_prompt(&multi_registry(), MemoryMode::Off, false).expect("a prompt");
    assert!(
        prompt.contains("Giving up with a reason"),
        "multi-connection prompt must also coach giving up: {prompt}"
    );
}

/// The single largest class of benchmark failures was the right numbers in the
/// wrong presentation, so the assembled prompt must brief the model on the
/// shape of an answer. The contract is a section in its own right, pushed
/// unconditionally alongside [`WORKING_GUIDANCE`].
#[test]
fn assembled_prompt_contains_the_answer_contract() {
    let prompt =
        assemble_system_prompt(&single_registry("main"), MemoryMode::Off, false).expect("a prompt");
    assert!(
        prompt.contains(ANSWER_CONTRACT),
        "the answer contract must be part of every prompt: {prompt}"
    );
}

/// The contract governs how the model writes any answer, so it is neither a
/// memory concern nor a multi-connection concern: it must appear in every
/// assembled prompt regardless of how many databases are connected or whether
/// memory is on, reachable, or off.
#[test]
fn answer_contract_present_regardless_of_connections_and_memory() {
    let cases: [(ConnectionRegistry, MemoryMode, bool); 6] = [
        (single_registry("main"), MemoryMode::Off, false),
        (single_registry("main"), MemoryMode::Assisted, true),
        (single_registry("main"), MemoryMode::Assisted, false),
        (multi_registry(), MemoryMode::Off, false),
        (multi_registry(), MemoryMode::Assisted, true),
        (multi_registry(), MemoryMode::Off, true),
    ];
    for (reg, mode, reachable) in cases {
        let prompt = assemble_system_prompt(&reg, mode, reachable).expect("a prompt for this case");
        assert!(
            prompt.contains(ANSWER_CONTRACT),
            "answer contract missing for memory {}, reachable {reachable}: {prompt}",
            mode.as_str(),
        );
    }
}

/// Stated ceiling on the answer-contract section. The contract rides on every
/// request, so its length is a real cost; this number keeps the section from
/// growing unbounded later. A new clause that crosses it must either tighten
/// the wording or raise the ceiling deliberately.
const ANSWER_CONTRACT_MAX_BYTES: usize = 1200;

#[test]
fn answer_contract_section_stays_under_documented_ceiling() {
    assert!(
        ANSWER_CONTRACT.len() <= ANSWER_CONTRACT_MAX_BYTES,
        "answer contract is {} bytes; the stated ceiling is {}",
        ANSWER_CONTRACT.len(),
        ANSWER_CONTRACT_MAX_BYTES,
    );
}

// --- prompt-cache prefix: the system block must be session-stable ----------
//
// A prompt cache is a *prefix* cache: providers reuse the longest leading run
// of tokens they have seen before. The previous turn's SQL was appended to the
// system prompt, so every follow-up that ran SQL changed the system block and
// forfeited the cached prefix. These tests pin the fix: the hint moves onto the
// user turn, and the system block stops varying with the previous SQL.

/// The assembled system prompt must not contain the previous turn's SQL, for
/// any input — the hint now lives on the user turn, so the system block is the
/// same whether or not a query just ran. `assemble_system_prompt` takes no SQL,
/// so the hint prose can never reach it; this locks that property.
#[test]
fn system_prompt_does_not_contain_the_previous_sql() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, MemoryMode::Off, true).expect("a prompt");
    assert!(
        !prompt.contains("most recent SQL you ran was"),
        "the SQL hint prose must not appear in the system prompt: {prompt}"
    );
    // The hint block carries the SQL; the system prompt never sees it.
    let hint = last_sql_hint_block("SELECT 1 FROM tbl").expect("a hint block");
    assert!(
        !prompt.contains(&hint.body),
        "the system prompt must not contain the hint block body: {prompt}"
    );
}

/// Two turns that differ only in the previous SQL must produce byte-identical
/// system *messages* (the constant SAYA prompt plus the assembled extra). This
/// is the property the prefix cache depends on, asserted directly through the
/// same `build_messages` the runtime uses, rather than inferred from the hint's
/// absence.
#[test]
fn system_message_is_byte_identical_across_turns_differing_only_in_last_sql() {
    let reg = single_registry("main");
    let system_prompt = assemble_system_prompt(&reg, MemoryMode::Off, true);
    let question = "refine the last query";
    // Turn A: no previous SQL. Turn B: a previous SQL hint on the user turn.
    let blocks_a: Vec<saya_agent::ContextBlock> = Vec::new();
    let blocks_b = vec![last_sql_hint_block("SELECT 1 FROM tbl").expect("a hint block")];
    let msgs_a = build_messages(
        system_prompt.as_deref(),
        &blocks_a,
        question,
        &[],
        32 * 1024,
    )
    .unwrap();
    let msgs_b = build_messages(
        system_prompt.as_deref(),
        &blocks_b,
        question,
        &[],
        32 * 1024,
    )
    .unwrap();
    assert_eq!(
        msgs_a[0], msgs_b[0],
        "the system message must not vary with the previous turn's SQL — that is what makes the prefix cache work"
    );
    // Sanity: the user turns do differ (B carries the hint).
    assert_ne!(msgs_a[1], msgs_b[1]);
}

/// The dialect statement must be present and name the connected engine: one
/// SQLite database was a PostgreSQL dump whose DDL still declared `jsonb` and
/// `point`, so the model wrote Postgres syntax SQLite rejected. The prompt must
/// say plainly that the SQL dialect is the connected engine's, whatever the DDL
/// declares.
#[test]
fn dialect_statement_names_the_connected_engine() {
    let prompt = assemble_system_prompt(&registry_with(SqlDialect::Sqlite), MemoryMode::Off, false)
        .expect("a prompt");
    assert!(
        prompt.contains("The SQL dialect is sqlite's, whatever the DDL says"),
        "the dialect statement must name the connected engine: {prompt}"
    );
    assert!(
        prompt.contains("declared column types in this database may come from another engine"),
        "the prompt must warn that declared types may come from another engine: {prompt}"
    );
}

/// The hint still reaches the model — on the user turn, never the system
/// message. The block is untrusted data rendered into the user turn by
/// `build_messages`, so the SQL appears beside the question and the system
/// message stays invariant.
#[test]
fn last_sql_hint_reaches_user_turn_not_system_message() {
    let reg = single_registry("main");
    let system_prompt = assemble_system_prompt(&reg, MemoryMode::Off, true);
    let hint = last_sql_hint_block("SELECT 1 FROM tbl").expect("a hint block");
    let messages = build_messages(
        system_prompt.as_deref(),
        &[hint],
        "refine it",
        &[],
        32 * 1024,
    )
    .unwrap();
    // The system message is invariant: no hint, no SQL.
    assert_eq!(messages[0].role, "system");
    assert!(
        !messages[0].content.contains("SELECT 1 FROM tbl"),
        "the hint must not leak into the system message: {messages:?}"
    );
    assert!(
        !messages[0].content.contains("most recent SQL you ran was"),
        "the hint prose must not leak into the system message: {messages:?}"
    );
    // The user turn carries the hint, labelled and escaped, before the question.
    assert_eq!(messages[1].role, "user");
    assert!(messages[1].content.contains("SELECT 1 FROM tbl"));
    assert!(messages[1].content.contains("most recent SQL you ran was"));
    assert!(
        messages[1].content.ends_with("refine it"),
        "the user's question must still trail the block: {messages:?}"
    );
}

/// The hint block is `None` for empty/whitespace SQL, so a turn with no prior
/// query adds no block and the user turn is byte-identical to one with no hint.
#[test]
fn last_sql_hint_block_is_none_for_empty_sql() {
    assert!(last_sql_hint_block("").is_none());
    assert!(last_sql_hint_block("   \n\t ").is_none());
    let hint = last_sql_hint_block("SELECT 1").expect("non-empty SQL yields a block");
    assert_eq!(hint.label, LAST_SQL_BLOCK_LABEL);
    assert!(hint.body.contains("SELECT 1"));
    assert!(hint.body.contains("most recent SQL you ran was"));
    assert!(!hint.truncated);
}
