use super::super::session_facts::{SESSION_FACTS_HEADING, SessionFacts, session_facts_text};
use super::super::turn_context::LAST_SQL_BLOCK_LABEL;
use super::*;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use async_trait::async_trait;
use saya_agent::{build_messages, turn_bytes};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use std::path::PathBuf;

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

#[path = "system_prompt_tests/assembly.rs"]
mod assembly;

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

#[path = "system_prompt_tests/engine_naming.rs"]
mod engine_naming;

#[path = "system_prompt_tests/answer_contract.rs"]
mod answer_contract;

/// Stated ceiling on the answer-contract section. The contract rides on every
/// request, so its length is a real cost; this number keeps the section from
/// growing unbounded later. A new clause that crosses it must either tighten
/// the wording or raise the ceiling deliberately.
const ANSWER_CONTRACT_MAX_BYTES: usize = 1200;

// --- prompt-cache prefix: the system block must be session-stable ----------
//
// A prompt cache is a *prefix* cache: providers reuse the longest leading run
// of tokens they have seen before. The previous turn's SQL was appended to the
// system prompt, so every follow-up that ran SQL changed the system block and
// forfeited the cached prefix. These tests pin the fix: the hint moves onto the
// user turn, and the system block stops varying with the previous SQL.

#[path = "system_prompt_tests/last_sql.rs"]
mod last_sql;

// --- mode slice 5: what the model is told under Plan -----------------------
//
// Plan is enforced (hidden definitions, engine Deny, derived permits off),
// advertised (absent from the tool list), reachable (`/mode`), visible
// (the status segment) and durable — but the model was never told what Plan
// means, so it could promise edits it cannot make. The fix is one appended
// paragraph under Plan only. These tests pin: Build byte-identical to before,
// Plan carrying the paragraph exactly once, and the append being the only
// difference.

use saya_agent::AgentMode;

#[path = "system_prompt_tests/plan_mode.rs"]
mod plan_mode;

/// Slice 3 — session facts: what the session is, not what it may do.
///
/// The system prompt must orient the model — which connections are in scope,
/// whether a workspace root is bound — without restating the tool list. The
/// facts are session-stable inputs (the connection set, the bound root), so
/// the section keeps one prefix-cache key across turns. Approval mode is out:
/// it can change mid-session via `/approvals`, which would churn the key.
fn facts<'a>(reg: &'a ConnectionRegistry, root: &'a Option<PathBuf>) -> SessionFacts<'a> {
    SessionFacts {
        registry: reg,
        workspace_root: root.as_deref(),
    }
}

#[path = "system_prompt_tests/session_facts.rs"]
mod session_facts;
