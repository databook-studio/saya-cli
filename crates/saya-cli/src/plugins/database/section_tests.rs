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
use crate::agent::session_facts::SessionFacts;
use crate::agent::system_prompt::assemble_system_prompt_with_session;
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
        super::tests::chat_session(),
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

/// Test 3 — one copy of each text: the plugin's section is composed of the same
/// bytes, in the same order, the old chat prompt carries, so the goldens cannot
/// drift when the chat switches to the plugin's section.
#[test]
fn the_section_is_the_old_chat_prompts_database_text() {
    let registry = sqlite_registry("local");
    let session = SessionFacts {
        registry: &registry,
        workspace_root: None,
    };
    let old = assemble_system_prompt_with_session(&registry, MemoryMode::Assisted, true, &session)
        .expect("the chat prompt");
    let section = section(&registry, MemoryMode::Assisted);

    let pieces: Vec<&str> = section.split("\n\n").collect();
    assert_eq!(
        pieces.len(),
        5,
        "memory, the database fact line, the guidance, the contract, the dialect: {pieces:?}"
    );
    let mut cursor = 0;
    for piece in pieces {
        let found = old[cursor..].find(piece).unwrap_or_else(|| {
            panic!("the old chat prompt must carry this section verbatim: {piece}\n\nin: {old}")
        });
        cursor += found + piece.len();
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
