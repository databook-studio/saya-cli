//! The database family on evidence: a bound profile alone is not a request to
//! use it.
//!
//! Every test here states the run's own evidence — its connections, the object
//! names cached from their schema, and the request's own words — and then reads
//! what the plugin advertises: the one ask before the latch, the whole family
//! after it, and never a byte of the database section until then.

use std::sync::Arc;

use saya_agent::{RunContext, RunPlugin, RunSurface};
use saya_config::MemoryMode;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

use super::family::{Bound, Family, USE_DATABASE};
use super::prompt::ChatPrompt;
use super::{ChatDatabaseFlags, DatabasePlugin};
use crate::agent::tools::DatabaseTools;
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// A chat run with a bound profile: the surface the rule is measured on.
const BOUND: RunContext = RunContext {
    surface: RunSurface::Chat,
    database_profiles: 1,
};

/// A connector that answers nothing: the section only reads the dialect.
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

/// The run's own registry: one SQLite connection named `local`, which is what
/// the section's dialect statement is rendered from.
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

/// The one connection every test's run holds.
fn local() -> Bound {
    Bound::new(
        vec![(String::from("local"), SqlDialect::Sqlite)],
        Vec::new(),
    )
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

/// Test 1 — a request that shows no database work is offered the one ask and
/// nothing else, and states no database section at all.
#[test]
fn a_request_without_database_work_advertises_only_the_ask() {
    let family = Arc::new(Family::of(local(), false));
    let plugin = chat(Arc::clone(&family));

    assert_eq!(
        names(&plugin),
        vec![USE_DATABASE.to_owned()],
        "a bound profile alone advertises the ask, never the family"
    );
    assert!(
        plugin.system_section(&BOUND).is_none(),
        "a request that never mentioned a database pays no database prompt bytes"
    );
}

/// Test 2 — a database request latches the tools and the section on its first
/// turn.
#[test]
fn a_database_request_latches_on_its_first_turn() {
    let family = Arc::new(Family::of(local(), false));
    let plugin = chat(Arc::clone(&family));

    assert!(
        family.note("what was our total revenue per month in 2024?"),
        "the request's own words state database work, so the family is on"
    );
    let advertised = names(&plugin);
    assert!(
        advertised.contains(&String::from("schema_discovery")),
        "the family is advertised on the first turn that asked: {advertised:?}"
    );
    let section = plugin
        .system_section(&BOUND)
        .expect("the database section rides the first turn that asked");
    assert!(
        section.contains("The SQL dialect is sqlite's"),
        "the section is the database part of the prompt: {section}"
    );
}

/// Test 3 — a natural question with no database word in it: the model calls
/// `use_database`, and the next request carries the family.
#[test]
fn the_model_can_ask_and_the_next_request_carries_the_family() {
    let family = Arc::new(Family::of(local(), false));
    let plugin = chat(Arc::clone(&family));

    assert!(
        !family.note("how did we do last quarter"),
        "a question that names no database is not evidence"
    );
    assert_eq!(names(&plugin), vec![USE_DATABASE.to_owned()]);

    let executor = plugin
        .executor()
        .expect("the chat surface serves its tools");
    let answer = futures_answer(executor.as_ref());
    assert!(
        answer.contains("local (sqlite)"),
        "the ask answers with the bound connections and their engines: {answer}"
    );

    let advertised = names(&plugin);
    assert!(
        advertised.contains(&String::from("bounded_sql_query")),
        "the request after the ask carries the family: {advertised:?}"
    );
    assert!(
        plugin.system_section(&BOUND).is_some(),
        "and the section rides with it"
    );
}

/// Answers a `use_database` call on `executor` through the same door the loop
/// uses, and states the result as text.
fn futures_answer(executor: &dyn saya_agent::ToolExecutor) -> String {
    let answer = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a test runtime")
        .block_on(executor.execute(USE_DATABASE, serde_json::json!({})))
        .expect("the ask is answered");
    answer.to_string()
}

/// Test 4 — the latch is the session's: a later request that says nothing about
/// a database keeps the family on.
#[test]
fn the_latch_persists_for_the_session() {
    let family = Arc::new(Family::of(local(), false));
    let plugin = chat(Arc::clone(&family));

    assert!(!family.latched(), "a bound profile alone is not the family");
    assert!(family.note("which table holds the orders?"));
    let after = names(&plugin);

    assert!(
        family.note("thanks — and the same for last year"),
        "a statement that shows no database work does not turn the family off"
    );
    assert_eq!(
        names(&plugin),
        after,
        "the advertisement is the same list on every later turn"
    );
}

/// Test 5 — the latch only appends: the ask keeps its place at the head of the
/// list, so the prefix a provider caches holds across the one change.
#[test]
fn the_tool_list_prefix_holds_across_the_latch() {
    let family = Arc::new(Family::of(local(), false));
    let plugin = chat(Arc::clone(&family));

    let before = names(&plugin);
    assert_eq!(before.first().map(String::as_str), Some(USE_DATABASE));

    assert!(family.note("select count(*) from orders"));
    let after = names(&plugin);

    assert_eq!(
        after.get(..before.len()),
        Some(before.as_slice()),
        "every name the run already advertised keeps its position: {after:?}"
    );
    assert!(
        after.len() > before.len(),
        "the family is appended after the ask, never before it: {after:?}"
    );
}

/// Test 6 — `[tools] allow` naming a database tool is the explicit override: the
/// family is on from the first turn, whatever the request says.
#[test]
fn a_stated_allow_list_turns_the_family_on() {
    let family = Arc::new(Family::of(local(), true));
    let plugin = chat(Arc::clone(&family));

    assert!(
        family.latched(),
        "the stated list names a database tool, so the family is on already"
    );
    let advertised = names(&plugin);
    assert!(
        advertised.contains(&String::from("schema_discovery")),
        "the family is advertised from the start: {advertised:?}"
    );

    // And without the stated name, the very same request advertises the ask
    // alone — the override is the list, not something else about the run.
    let plain = chat(Arc::new(Family::of(local(), false)));
    assert_eq!(names(&plain), vec![USE_DATABASE.to_owned()]);
}

/// Test 7 — no bound profile: nothing database-shaped is advertised, and there
/// is no ask either.
#[test]
fn no_profile_advertises_nothing_database_shaped() {
    let family = Arc::new(Family::of(Bound::default(), false));
    let plugin = chat(Arc::clone(&family));

    assert!(
        names(&plugin).is_empty(),
        "there is no database to ask about"
    );
    assert!(!family.note("show me the revenue by month"));
    assert!(plugin.system_section(&BOUND).is_none());
}

/// Test 8 — the bound profile's cached schema is evidence too: a request that
/// names one of its tables is database work without saying "database". The two
/// requests here name no word and no phrase of [`super::evidence`], so only the
/// cached name can answer for them.
#[test]
fn a_name_from_the_cached_schema_is_database_work() {
    let bound = Bound::new(
        vec![(String::from("local"), SqlDialect::Sqlite)],
        vec![String::from("orders"), String::from("customer_id")],
    );
    let family = Arc::new(Family::of(bound, false));
    let plugin = chat(Arc::clone(&family));

    assert!(
        family.note("compare the widgets against orders"),
        "a table the profile's schema holds is database work"
    );
    assert!(
        names(&plugin).contains(&String::from("schema_discovery")),
        "so the family is advertised"
    );

    let fresh = Arc::new(Family::of(local(), false));
    assert!(
        !fresh.note("compare the widgets against gnomes"),
        "a word no cached schema names is not evidence"
    );
}

/// Test 9 — a bound connection's name is evidence: naming the database the
/// session holds is asking for it.
#[test]
fn a_bound_connection_name_is_database_work() {
    let family = Arc::new(Family::of(local(), false));

    assert!(!family.latched(), "a bound profile alone is not the family");
    assert!(
        family.note("compare local with the figures I pasted"),
        "the run's own connection name is evidence"
    );
}

/// Test 10 — a verb form and the engine's own name are database work too: a
/// request that says `querying`, or names the SQLite the run is bound to, is
/// asking for the database without saying "database".
#[test]
fn a_verb_form_or_the_engines_name_is_database_work() {
    let asking = Arc::new(Family::of(local(), false));
    assert!(asking.note("why is querying the monthly figures slow?"));

    let engine = Arc::new(Family::of(local(), false));
    assert!(engine.note("remember the SQLite answer after checking it"));

    let plain = Arc::new(Family::of(local(), false));
    assert!(
        !plain.note("why is the monthly summary slow?"),
        "a word that merely starts with one of the fourteen is not evidence"
    );

    let plural = Arc::new(Family::of(local(), false));
    assert!(
        plural.note("inspect both databases"),
        "the plural of a word is the word"
    );
}

/// Test 11 — the question a session actually asks: "how many orders are there"
/// is database work on its own, with an empty schema cache and nothing cached
/// to name `orders`. A request that asks for neither counts nothing.
#[test]
fn the_common_counting_question_latches_on_its_first_turn() {
    let family = Arc::new(Family::of(local(), false));
    let plugin = chat(Arc::clone(&family));

    assert!(
        family.note("how many orders are there"),
        "the counting question is database work with no cached schema"
    );
    let advertised = names(&plugin);
    assert!(
        advertised.contains(&String::from("schema_discovery")),
        "so the family rides the turn that asked: {advertised:?}"
    );

    let typo = Arc::new(Family::of(local(), false));
    assert!(
        !typo.note("fix the typo in README"),
        "a request about neither is still not evidence"
    );
}
