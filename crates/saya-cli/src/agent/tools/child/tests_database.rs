//! The coding database surface: the standard read-only database tools
//! (`schema_discovery`, `bounded_sql_query`) over the job's admitted profiles,
//! on the root and on a helper alike, through the same read-only safety layer
//! the chat agent's tools use. A write statement is the safety layer's to
//! refuse; an unselected profile is refused before any connection exists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{ToolError, ToolExecutionContext};
use saya_store::SqliteStateStore;
use saya_types::{DatabaseProfile, ProfileIdentity};

use super::CliChildFactory;
use super::tests::{Sandbox, checkout, database_view, profile_identity};
use crate::commands::coding::database::CodingDatabaseTools;
use crate::config::runtime::RuntimeConfig;

/// A resolved runtime with no configured profile: the tests insert exactly the
/// profiles they select, so nothing else can be reached.
fn runtime() -> RuntimeConfig {
    let root = std::env::temp_dir().join(format!("saya-coding-db-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut runtime = crate::load_with_sources(
        &crate::GlobalOptions::default(),
        &root,
        &root,
        BTreeMap::new(),
    )
    .unwrap();
    // The data-sharing gate is the run's resolved policy; these tests are about
    // the toolset and the SQL safety layer, so it is on.
    runtime.resolved.ai.allow_data_sharing = true;
    runtime
}

fn store() -> SqliteStateStore {
    SqliteStateStore::new(
        std::env::temp_dir().join(format!("saya-coding-db-{}.sqlite3", std::process::id())),
    )
}

/// An absolute, read/write SQLite profile whose file exists: the connection is
/// writable, so a write statement can only be refused by the safety layer.
fn sqlite_profile(root: &Path, name: &str) -> (DatabaseProfile, PathBuf) {
    let path = root.join(format!("{name}.sqlite3"));
    std::fs::File::create(&path).unwrap();
    let profile = DatabaseProfile::Sqlite {
        path: path.to_str().unwrap().to_owned(),
        read_only: false,
    };
    (profile, path)
}

/// A runtime that carries exactly `profiles`.
fn runtime_with(profiles: &[(String, DatabaseProfile)]) -> RuntimeConfig {
    let mut runtime = runtime();
    for (name, profile) in profiles {
        runtime
            .connections
            .profiles
            .insert(name.clone(), profile.clone());
    }
    runtime
}

fn selected(entries: &[(String, ProfileIdentity)]) -> BTreeMap<String, ProfileIdentity> {
    entries.iter().cloned().collect()
}

fn advertises(definitions: &[saya_agent::ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|definition| definition.name == name)
}

fn factory_with(
    sandbox: &Sandbox,
    checkout_id: &saya_types::ManagedCheckoutId,
    databases: CodingDatabaseTools,
) -> CliChildFactory {
    CliChildFactory::new(
        sandbox.pair(checkout_id),
        super::tests_factory::test_ai(),
        Arc::new(saya_config::MapSecretResolver::new([])),
    )
    .with_databases(databases)
}

fn context() -> ToolExecutionContext {
    ToolExecutionContext {
        result_cap: 64 * 1024,
        cancellation: saya_agent::CancellationToken::new(),
    }
}

/// The `connection` property's enum, as the model is told it.
fn connection_enum(definitions: &[saya_agent::ToolDefinition], name: &str) -> Vec<String> {
    definitions
        .iter()
        .find(|definition| definition.name == name)
        .expect("the tool is advertised")
        .parameters["properties"]["connection"]["enum"]
        .as_array()
        .expect("the connection property enumerates the admitted profiles")
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect()
}

/// A helper advertises the standard read-only database tools — never a
/// `coding_*` alias — when the job selected a profile its view admits, and the
/// `connection` enum names exactly the admitted profile.
#[tokio::test]
async fn a_helper_advertises_the_standard_database_tools() {
    let checkout_id = checkout("checkout");
    let root =
        std::env::temp_dir().join(format!("saya-coding-db-advertise-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let (profile, _path) = sqlite_profile(&root, "main");
    let runtime = runtime_with(&[("main".to_owned(), profile.clone())]);
    let identity = profile_identity("main", &profile, &runtime.cache_scope);
    let view = database_view(&checkout_id, vec![identity.clone()]);
    let sandbox = Sandbox::new("db-advertise");
    let factory = factory_with(
        &sandbox,
        &checkout_id,
        CodingDatabaseTools::new(
            runtime,
            store(),
            selected(&[("main".to_owned(), identity)]),
            false,
        ),
    );

    let (definitions, _tools, _approval) = factory.build_toolset(&view).unwrap();

    assert!(advertises(&definitions, "schema_discovery"));
    assert!(advertises(&definitions, "bounded_sql_query"));
    assert_eq!(connection_enum(&definitions, "schema_discovery"), ["main"]);
    assert_eq!(connection_enum(&definitions, "bounded_sql_query"), ["main"]);
    // Exactly the two standard tools: the rest of the chat toolset (the
    // chart, contract and fan-out tools) stays off the coding surface, and no
    // `coding_*` alias survives. The ask is no database tool, so it is not here.
    let mut database_tools: Vec<&str> = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .filter(|name| {
            !crate::agent::tools::workspace::is_tool_name(name)
                && *name != "checklist"
                && *name != "helpers"
        })
        .collect();
    database_tools.sort_unstable();
    assert_eq!(database_tools, ["bounded_sql_query", "schema_discovery"]);
}

/// A job that selected no profile advertises no database tool at all — not an
/// empty-enum one.
#[tokio::test]
async fn a_job_without_profiles_advertises_no_database_tool() {
    let checkout_id = checkout("checkout");
    let view = database_view(&checkout_id, Vec::new());
    let sandbox = Sandbox::new("db-none");
    let factory = factory_with(
        &sandbox,
        &checkout_id,
        CodingDatabaseTools::new(runtime(), store(), BTreeMap::new(), false),
    );

    let (definitions, _tools, _approval) = factory.build_toolset(&view).unwrap();

    assert!(!advertises(&definitions, "schema_discovery"));
    assert!(!advertises(&definitions, "bounded_sql_query"));
}

/// A profile the job selected but this view does not admit is neither
/// advertised nor reachable: the definition names only the admitted profile and
/// the call is refused before any connection exists.
#[tokio::test]
async fn an_unadmitted_profile_is_not_advertised_and_is_refused() {
    let checkout_id = checkout("checkout");
    let root =
        std::env::temp_dir().join(format!("saya-coding-db-unselected-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let (main, _main_path) = sqlite_profile(&root, "main");
    let (other, _other_path) = sqlite_profile(&root, "other");
    let runtime = runtime_with(&[
        ("main".to_owned(), main.clone()),
        ("other".to_owned(), other.clone()),
    ]);
    let main_identity = profile_identity("main", &main, &runtime.cache_scope);
    let other_identity = profile_identity("other", &other, &runtime.cache_scope);
    let view = database_view(&checkout_id, vec![main_identity.clone()]);
    let databases = CodingDatabaseTools::new(
        runtime,
        store(),
        selected(&[
            ("main".to_owned(), main_identity),
            ("other".to_owned(), other_identity),
        ]),
        false,
    );

    let definitions = databases.definitions(Some(&view));
    assert_eq!(connection_enum(&definitions, "bounded_sql_query"), ["main"]);

    let refused = databases
        .execute(
            Some(&view),
            "bounded_sql_query",
            serde_json::json!({"connection": "other", "sql": "SELECT 1"}),
            context(),
        )
        .await
        .expect_err("an unadmitted profile must be refused");
    assert!(matches!(refused, ToolError::NoEffect(_)), "{refused}");
    assert!(refused.to_string().contains("not approved"), "{refused}");
}

/// A profile the job never selected at all is refused the same way.
#[tokio::test]
async fn an_unselected_profile_is_refused() {
    let checkout_id = checkout("checkout");
    let root = std::env::temp_dir().join(format!(
        "saya-coding-db-never-selected-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let (main, _main_path) = sqlite_profile(&root, "main");
    let runtime = runtime_with(&[("main".to_owned(), main.clone())]);
    let identity = profile_identity("main", &main, &runtime.cache_scope);
    let view = database_view(&checkout_id, vec![identity.clone()]);
    let databases = CodingDatabaseTools::new(
        runtime,
        store(),
        selected(&[("main".to_owned(), identity)]),
        false,
    );

    let refused = databases
        .execute(
            Some(&view),
            "bounded_sql_query",
            serde_json::json!({"connection": "elsewhere", "sql": "SELECT 1"}),
            context(),
        )
        .await
        .expect_err("a profile the job never selected must be refused");
    assert!(matches!(refused, ToolError::NoEffect(_)), "{refused}");
}

/// The read-only safety layer refuses a write statement and serves a read: the
/// coding surface is the chat agent's own toolset, so the SQL layer is the one
/// that decides.
#[tokio::test]
async fn a_write_statement_is_refused_by_the_read_only_safety_layer() {
    let checkout_id = checkout("checkout");
    let root = std::env::temp_dir().join(format!("saya-coding-db-write-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let (main, _main_path) = sqlite_profile(&root, "main");
    let runtime = runtime_with(&[("main".to_owned(), main.clone())]);
    let identity = profile_identity("main", &main, &runtime.cache_scope);
    let view = database_view(&checkout_id, vec![identity.clone()]);
    let databases = CodingDatabaseTools::new(
        runtime,
        store(),
        selected(&[("main".to_owned(), identity)]),
        false,
    );

    let refused = databases
        .execute(
            Some(&view),
            "bounded_sql_query",
            serde_json::json!({
                "connection": "main",
                "sql": "CREATE TABLE written (id INTEGER)"
            }),
            context(),
        )
        .await
        .expect_err("a write statement must be refused");
    assert!(
        refused.to_string().contains("read-only safety policy"),
        "the safety layer refuses it: {refused}"
    );

    let served = databases
        .execute(
            Some(&view),
            "bounded_sql_query",
            serde_json::json!({"connection": "main", "sql": "SELECT 1 AS one"}),
            context(),
        )
        .await
        .expect("a read is served");
    assert_eq!(served["row_count"], 1, "{served}");
    assert_eq!(served["executed_sql"], "SELECT 1 AS one");
}
