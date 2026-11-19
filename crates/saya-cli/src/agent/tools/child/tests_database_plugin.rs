//! The job's database tools reach a factory-built executor through the
//! database plugin: one owner for those names, and one door the call travels.
//! The bytes the model reads are the plugin's, so the test pins the door — a
//! built executor answers a call the plugin owns — as well as the answer.

use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::ToolExecutor;

use super::CliChildFactory;
use super::tests::{Sandbox, checkout, database_view, profile_identity};
use super::tests_database::{runtime_with, selected, sqlite_profile, store};
use crate::commands::coding::database::CodingDatabaseTools;

/// A scratch directory under the workspace's own `target/` — never the system
/// temp directory.
fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-child-database-plugin-{name}"));
    std::fs::create_dir_all(&root).expect("scratch directory");
    root
}

fn advertises(definitions: &[saya_agent::ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|definition| definition.name == name)
}

/// A job that selected `main` advertises the standard database tools, and the
/// executor the factory built serves them — through the plugin that owns them.
#[tokio::test]
async fn a_factory_built_executor_serves_its_database_tools_through_the_plugin() {
    let checkout_id = checkout("checkout");
    let root = scratch("serve");
    let (profile, _path) = sqlite_profile(&root, "main");
    let resolved = runtime_with(&[("main".to_owned(), profile.clone())]);
    let identity = profile_identity("main", &profile, &resolved.cache_scope);
    let view = database_view(&checkout_id, vec![identity.clone()]);
    let sandbox = Sandbox::new("db-plugin");
    let factory = CliChildFactory::new(
        sandbox.pair(&checkout_id),
        super::tests_factory::test_ai(),
        Arc::new(saya_config::MapSecretResolver::new([])),
    )
    .with_databases(CodingDatabaseTools::new(
        resolved,
        store(),
        selected(&[("main".to_owned(), identity)]),
        false,
    ));

    let (definitions, tools, _approval) = factory.build_toolset(&view).unwrap();
    assert!(advertises(&definitions, "bounded_sql_query"));

    let served = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"connection": "main", "sql": "SELECT 1 AS one"}),
        )
        .await
        .expect("the plugin serves the job's database tools");
    assert_eq!(served["row_count"], 1, "{served}");
    assert_eq!(served["executed_sql"], "SELECT 1 AS one");
}
