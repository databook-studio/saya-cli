//! The plugin's section of the run's system prompt: the database part of it,
//! byte for byte what the chat prompt states today.
//!
//! The section is per run, so every turn of a run sends the same bytes. It
//! carries the memory briefing, the session's database fact lines, the working
//! guidance, the answer contract, and the engine naming/dialect statement — and
//! it is the *same* text the old chat prompt renders, which is what keeps the
//! goldens from drifting when the chat switches over.

use std::sync::Arc;

use saya_agent::{PluginRegistry, RunContext, RunPlugin, RunSurface};
use saya_config::MemoryMode;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

use super::prompt::ChatPrompt;
use super::{ChatDatabaseFlags, DatabasePlugin};
use crate::agent::tools::DatabaseTools;
use crate::agent::turn_context::last_sql_hint_block;
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// A chat run's context: the surface has a database, so the plugin activates.
const CHAT: RunContext = RunContext {
    surface: RunSurface::Chat,
    database_profiles: 1,
};

/// The core tool names the plugin is registered beside. None is a database
/// tool, so the registry accepts the plugin on a real run's tool list.
const CORE_TOOLS: [&str; 3] = ["read", "write", "shell"];

/// A connector that answers nothing: the prompt only reads the dialect.
struct DummyConnector {
    dialect: SqlDialect,
}

#[async_trait::async_trait]
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
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(request.sql))
    }
}

/// One SQLite connection named `name` — the profile shape the chat runs with.
fn sqlite_registry(name: &str) -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new(name);
    let dialect = SqlDialect::Sqlite;
    registry.insert(
        name,
        ConnectionEntry {
            connector: Box::new(DummyConnector { dialect }),
            dialect,
            profile_id: None,
        },
    );
    registry
}

/// A chat-surface plugin whose prompt inputs are the run's resolved facts.
fn chat_plugin_with(registry: &ConnectionRegistry, memory: MemoryMode) -> DatabasePlugin {
    DatabasePlugin::for_chat(
        Arc::new(DatabaseTools::new(None, 100, true)),
        ChatDatabaseFlags {
            allow_query_data: true,
            has_state_store: true,
            permit_candidate_writes: true,
        },
        super::tests::chat_budget(),
        ChatPrompt::of(registry, memory, true),
        None,
    )
}

/// The section a chat plugin states, rendered from the same registry.
fn section(registry: &ConnectionRegistry, memory: MemoryMode) -> String {
    chat_plugin_with(registry, memory)
        .system_section(&CHAT)
        .expect("a chat run with a database states its database text")
}

/// Test 1 — the chat section is the database part of the prompt: the answer
/// contract, the dialect statement, and the session's database fact line. With
/// memory off it carries no memory paragraph.
#[test]
fn the_chat_section_states_the_database_text() {
    let registry = sqlite_registry("local");
    let off = section(&registry, MemoryMode::Off);

    assert!(
        off.contains("Answer the question exactly as asked"),
        "the answer contract rides the section: {off}"
    );
    assert!(
        off.contains("The SQL dialect is sqlite's"),
        "the dialect line names the engine: {off}"
    );
    assert!(
        off.contains("Database `local` (sqlite)"),
        "the session's database fact line rides the section: {off}"
    );
    assert!(
        !off.contains("durable knowledge"),
        "memory off states no memory paragraph: {off}"
    );

    let assisted = section(&registry, MemoryMode::Assisted);
    assert!(
        assisted.contains("durable knowledge"),
        "memory assisted states the memory paragraph: {assisted}"
    );
}

/// Test 2 — a chat run with no database profile activates no database plugin,
/// so the registry asks it for no section at all: zero bytes.
#[test]
fn a_run_without_a_database_is_asked_for_no_section() {
    let registry = sqlite_registry("local");
    let plugins = PluginRegistry::new(
        vec![Arc::new(chat_plugin_with(&registry, MemoryMode::Off))],
        &CORE_TOOLS,
    )
    .expect("no core tool is a database tool");

    let without = plugins.active(&RunContext {
        surface: RunSurface::Chat,
        database_profiles: 0,
    });
    assert!(
        without.system_sections().is_empty(),
        "a chat run with no profile contributes no database text"
    );

    let with = plugins.active(&CHAT);
    let sections = with.system_sections();
    assert_eq!(sections.len(), 1, "one active plugin, one section");
    for word in ["Database", "dialect", "schema"] {
        assert!(
            sections[0].contains(word),
            "an active plugin's section states `{word}`: {}",
            sections[0]
        );
    }
}

/// Test 3 — the section rides every request of a run that has latched the
/// family, so its bytes are capped: `perf_guard::DB_SECTION_CAP` is today's
/// section with one connection and memory on, plus a 5 % headroom. A paragraph
/// added here is paid on every turn of every database session.
#[test]
fn the_section_fits_its_byte_cap() {
    let registry = sqlite_registry("local");
    for memory in [MemoryMode::Off, MemoryMode::Assisted] {
        let text = section(&registry, memory);
        println!(
            "database section: {} bytes with memory {memory:?}",
            text.len()
        );
        assert!(
            text.len() <= crate::perf_guard::DB_SECTION_CAP,
            "the database section is {} bytes with memory {memory:?}, over the {}-byte cap in \
             `perf_guard.rs`: shorten the section, or raise that cap in review with the reason",
            text.len(),
            crate::perf_guard::DB_SECTION_CAP
        );
    }
}

/// The context hook carries the adapt-this-query hint the old turn carried, in
/// the same block the turn built — and states none for a turn that ran no SQL.
#[tokio::test]
async fn the_context_hook_carries_the_last_sql_hint() {
    let registry = sqlite_registry("local");
    let plugin = chat_plugin_with(&registry, MemoryMode::Off);
    let sql = "SELECT SUM(amount) AS total FROM revenue";

    plugin.record_last_sql(Some(sql));
    let blocks = plugin.context(&CHAT, "how much revenue is there").await;
    let expected = last_sql_hint_block(sql).expect("non-empty SQL yields a hint");
    assert_eq!(blocks.len(), 1, "one hint, no recall source attached");
    assert_eq!(blocks[0].label, expected.label, "the turn's block label");
    assert_eq!(blocks[0].body, expected.body, "the turn's block body");

    plugin.record_last_sql(None);
    assert!(
        plugin
            .context(&CHAT, "how much revenue is there")
            .await
            .is_empty(),
        "a turn that ran no SQL states nothing about one"
    );
}
