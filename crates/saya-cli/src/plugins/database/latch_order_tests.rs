//! The database family only ever appends to the tool list: the ask keeps its
//! place at the head, and the family follows it.
//!
//! A provider caches a request's prefix, so the one change a session makes to
//! its tool list — the latch — must leave every name already advertised where
//! it was. [`super::family_tests`] pins the rule's own turn-by-turn behaviour;
//! this module pins the order the whole plugin's list holds across the latch,
//! beside the web and helper gates (`agent::tools::coding::evidence_append_tests`).
//!
//! The database section rides the same latch and is capped by
//! [`super::section_tests`].

use std::sync::Arc;

use saya_agent::{RunContext, RunPlugin, RunSurface};
use saya_config::MemoryMode;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

use super::family::{Bound, Family};
use super::prompt::ChatPrompt;
use super::{ChatDatabaseFlags, DatabasePlugin};
use crate::agent::tools::DatabaseTools;
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// A chat run with a bound profile: the surface the latch is measured on.
const BOUND: RunContext = RunContext {
    surface: RunSurface::Chat,
    database_profiles: 1,
};

/// A connector that answers nothing: the prompt only reads the dialect.
struct Dummy {
    dialect: SqlDialect,
}

#[async_trait::async_trait]
impl saya_connectors::DatabaseConnector for Dummy {
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(request.sql))
    }
}

/// The run's one connection, named `local`.
fn registry() -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new("local");
    let dialect = SqlDialect::Sqlite;
    registry.insert(
        "local",
        ConnectionEntry {
            connector: Box::new(Dummy { dialect }),
            dialect,
            profile_id: None,
        },
    );
    registry
}

/// A chat plugin over `family`, holding the run's own toolset and prompt.
fn chat(family: Arc<Family>) -> DatabasePlugin {
    DatabasePlugin::for_chat(
        Arc::new(DatabaseTools::new(None, 100, true)),
        ChatDatabaseFlags {
            allow_query_data: true,
            has_state_store: true,
            permit_candidate_writes: true,
        },
        super::tests::chat_budget(),
        ChatPrompt::of(&registry(), MemoryMode::Off, false),
        None,
    )
    .with_family(family)
}

/// The names the plugin advertises, in composition order.
fn names(plugin: &DatabasePlugin) -> Vec<String> {
    plugin
        .definitions(&BOUND)
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}

/// Test 1 — the family appends: the list the plugin advertises before the latch
/// is a prefix of the list it advertises after it.
#[test]
fn the_database_family_latch_only_appends() {
    let family = Arc::new(Family::of(
        Bound::new(
            vec![(String::from("local"), SqlDialect::Sqlite)],
            Vec::new(),
        ),
        false,
    ));
    let plugin = chat(Arc::clone(&family));

    let before = names(&plugin);
    assert!(family.note("how many orders are there"));
    let after = names(&plugin);
    println!("before: {before:?}\nafter: {after:?}");

    assert_eq!(
        after.get(..before.len()),
        Some(before.as_slice()),
        "the family only appends: every name advertised before the latch keeps its position \
         (before {before:?}, after {after:?})"
    );
    assert!(
        after.len() > before.len(),
        "the family is advertised once latched: {after:?}"
    );
}
