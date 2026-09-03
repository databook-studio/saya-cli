use super::*;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use async_trait::async_trait;
use saya_agent::turn_bytes;
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
    let prompt = assemble_system_prompt(&reg, None, MemoryMode::Off, true).expect("a prompt");
    // With one connection and memory off there is no context and no briefing —
    // but the model is still shown a catalog/schema/table tree by schema
    // discovery, so it still has to be told what SQL will accept.
    assert!(!prompt.contains("durable knowledge"));
    assert!(prompt.contains("catalog.schema.object"));
}

#[test]
fn assemble_system_prompt_single_connection_assisted() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, None, MemoryMode::Assisted, true).expect("a prompt");
    assert!(prompt.starts_with(MEMORY_SYSTEM_PROMPT));
    // A single PostgreSQL connection: the memory briefing, then its naming rule.
    assert!(prompt.contains("catalog.schema.object"));
}

#[test]
fn assemble_system_prompt_with_last_sql_and_memory_off() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, Some("SELECT 1 FROM tbl"), MemoryMode::Off, true);
    assert!(prompt.is_some());
    let text = prompt.unwrap();
    assert!(text.contains("SELECT 1 FROM tbl"));
    assert!(!text.contains("durable knowledge"));
}

#[test]
fn assemble_system_prompt_with_last_sql_and_assisted() {
    let reg = single_registry("main");
    let prompt =
        assemble_system_prompt(&reg, Some("SELECT 1 FROM tbl"), MemoryMode::Assisted, true);
    assert!(prompt.is_some());
    let text = prompt.unwrap();
    assert!(text.contains(MEMORY_SYSTEM_PROMPT));
    assert!(text.contains("SELECT 1 FROM tbl"));
    assert!(text.contains("For context, the most recent SQL you ran was:"));
}

#[test]
fn assemble_system_prompt_multi_connection_assisted_and_sql() {
    let reg = multi_registry();
    let prompt = assemble_system_prompt(&reg, Some("SELECT 1"), MemoryMode::Assisted, true);
    assert!(prompt.is_some());
    let text = prompt.unwrap();

    // Contains all three sections in expected order
    let conn_idx = text.find("Available database connections").unwrap();
    let mem_idx = text.find("SAYA maintains durable knowledge").unwrap();
    let sql_idx = text
        .find("For context, the most recent SQL you ran was:")
        .unwrap();

    assert!(conn_idx < mem_idx);
    assert!(mem_idx < sql_idx);
}

#[test]
fn assemble_system_prompt_stays_within_budget() {
    let reg = multi_registry();
    let prompt = assemble_system_prompt(
        &reg,
        Some("SELECT * FROM complex_catalog.schema.large_table WHERE id = 123"),
        MemoryMode::Assisted,
        true,
    );
    assert!(prompt.is_some());
    let system_text = prompt.unwrap();

    // Check raw length is tiny compared to limit
    assert!(system_text.len() < 2000);

    // Check turn_bytes calculation passes budget
    let bytes = turn_bytes(Some(&system_text), &[], "How many orders were placed?");
    assert!(bytes < saya_types::MAX_MESSAGE_BYTES);
}

/// Assisted, but the state store did not open or the privacy gate is shut: the
/// section promises supplied facts and two tools that this turn does not have,
/// so it must not appear.
#[test]
fn memory_section_is_absent_when_memory_is_configured_on_but_unreachable() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, None, MemoryMode::Assisted, false).expect("a prompt");
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
        None,
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
        None,
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
        let prompt =
            assemble_system_prompt(&registry_with(dialect), None, MemoryMode::Assisted, true)
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
    let prompt = assemble_system_prompt(
        &registry_with(SqlDialect::Sqlite),
        None,
        MemoryMode::Off,
        false,
    )
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
