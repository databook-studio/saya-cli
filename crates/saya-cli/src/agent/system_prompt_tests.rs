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
    assert!(text.contains("fully qualified `catalog.schema.object`"));
}

#[test]
fn assemble_system_prompt_single_connection_off() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, None, MemoryMode::Off, true);
    assert_eq!(prompt, None);
}

#[test]
fn assemble_system_prompt_single_connection_assisted() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, None, MemoryMode::Assisted, true);
    assert_eq!(prompt, Some(MEMORY_SYSTEM_PROMPT.to_string()));
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
    let prompt = assemble_system_prompt(&reg, None, MemoryMode::Assisted, false);
    assert_eq!(prompt, None, "no section, and nothing else to say");

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
