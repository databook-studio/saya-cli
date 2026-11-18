//! The database plugin's contract: the definitions each surface offers today,
//! the declared-name list the registry checks, and the executor route those
//! names lead to.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{
    PluginError, PluginRegistry, RunContext, RunPlugin, RunSurface, ToolDefinition, ToolError,
};
use saya_store::SqliteStateStore;
use saya_types::DatabaseProfile;

use super::{ChatDatabaseFlags, DatabasePlugin};
use crate::agent::tools::DatabaseTools;
use crate::agent::tools::workspace::is_tool_name;
use crate::commands::coding::database::CodingDatabaseTools;
use crate::config::runtime::RuntimeConfig;

/// The core tool names the plugin is registered beside. None is a database
/// tool, so the registry accepts the plugin on a real run's tool list.
const CORE_TOOLS: [&str; 7] = ["read", "list", "glob", "grep", "write", "edit", "shell"];

/// A chat run's context: the surface has a database, so the plugin activates.
const CHAT: RunContext = RunContext {
    surface: RunSurface::Chat,
    database_profiles: 1,
};

fn chat_plugin() -> DatabasePlugin {
    DatabasePlugin::for_chat(
        Arc::new(DatabaseTools::new(None, 100, true)),
        ChatDatabaseFlags {
            allow_query_data: true,
            has_state_store: true,
            permit_candidate_writes: true,
        },
    )
}

/// A scratch directory under the workspace's own `target/` — never the system
/// temp directory.
fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-plugins-database-{name}"));
    std::fs::create_dir_all(&root).expect("scratch directory");
    root
}

/// A resolved runtime, data sharing on — the run's policy the job definitions
/// read — with no configured profile of its own.
fn runtime(root: &Path) -> RuntimeConfig {
    let mut runtime = crate::load_with_sources(
        &crate::GlobalOptions::default(),
        root,
        root,
        BTreeMap::new(),
    )
    .expect("a runtime resolves from an empty configuration");
    runtime.resolved.ai.allow_data_sharing = true;
    runtime
}

/// A coding job's database tools over exactly one selected SQLite profile.
fn job_tools(root: &Path, clarification: bool) -> CodingDatabaseTools {
    let path = root.join("main.sqlite3");
    std::fs::File::create(&path).expect("the profile's file exists");
    let profile = DatabaseProfile::Sqlite {
        path: path.to_str().expect("a UTF-8 path").to_owned(),
        read_only: false,
    };
    let mut runtime = runtime(root);
    runtime
        .connections
        .profiles
        .insert("main".to_owned(), profile.clone());
    let identity =
        crate::profile_identity::profile_identity("main", &profile, &runtime.cache_scope);
    CodingDatabaseTools::new(
        runtime,
        SqliteStateStore::new(root.join("state.sqlite3")),
        BTreeMap::from([("main".to_owned(), identity)]),
        clarification,
    )
}

fn names(definitions: &[ToolDefinition]) -> Vec<String> {
    definitions
        .iter()
        .map(|definition| definition.name.clone())
        .collect()
}

/// Byte-equal comparison: the model-facing definition is its serialized form,
/// so equality of the strings is the equality that matters.
fn serialized(definitions: &[ToolDefinition]) -> Vec<String> {
    definitions
        .iter()
        .map(|definition| serde_json::to_string(definition).expect("a definition serializes"))
        .collect()
}

/// Test 1 — the chat definitions are the non-workspace subset of the list the
/// chat builds today, in the same order and byte for byte.
#[test]
fn chat_definitions_are_the_non_workspace_subset() {
    let plugin = chat_plugin();
    let expected: Vec<ToolDefinition> = DatabaseTools::definitions(true, true, true, false)
        .into_iter()
        .filter(|definition| !is_tool_name(&definition.name))
        .collect();

    let actual = plugin.definitions(&CHAT);

    assert_eq!(names(&actual), names(&expected), "same tools, same order");
    assert_eq!(serialized(&actual), serialized(&expected), "byte-equal");
    assert!(
        actual
            .iter()
            .all(|definition| !is_tool_name(&definition.name)),
        "no workspace file tool is the plugin's: {:?}",
        names(&actual)
    );
}

/// Test 2 — a coding job's definitions are exactly what the job builds itself.
#[test]
fn job_definitions_are_the_jobs_own() {
    let root = scratch("job");
    let tools = Arc::new(job_tools(&root, true));
    let plugin = DatabasePlugin::for_job(tools.clone(), None);

    let actual = plugin.definitions(&CHAT);
    let expected = tools.definitions(None);

    assert_eq!(serialized(&actual), serialized(&expected), "byte-equal");
    assert!(
        names(&expected)
            .iter()
            .any(|name| name == "schema_discovery"),
        "the fixture offers a real database tool: {:?}",
        names(&expected)
    );
}

/// Test 3 — the declared list covers every name either surface can ever offer,
/// on every combination of the flags, and claims no workspace file tool.
#[test]
fn declared_tools_cover_every_definition_and_no_workspace_name() {
    let plugin = chat_plugin();
    let mut offered: Vec<String> = Vec::new();
    for allow_query_data in [false, true] {
        for has_state_store in [false, true] {
            for permit_candidate_writes in [false, true] {
                for permit_workspace_writes in [false, true] {
                    for definition in DatabaseTools::definitions(
                        allow_query_data,
                        has_state_store,
                        permit_candidate_writes,
                        permit_workspace_writes,
                    ) {
                        if !is_tool_name(&definition.name) {
                            offered.push(definition.name);
                        }
                    }
                }
            }
        }
    }
    let root = scratch("declared");
    for clarification in [false, true] {
        offered.extend(names(&job_tools(&root, clarification).definitions(None)));
    }

    for name in &offered {
        assert!(
            plugin.declared_tools().contains(&name.as_str()),
            "`{name}` is offered but never declared"
        );
    }
    for name in plugin.declared_tools() {
        assert!(
            !is_tool_name(name),
            "`{name}` is a workspace file tool, which is the core's"
        );
        assert!(
            offered.iter().any(|offered| offered == name),
            "`{name}` is declared but offered by neither surface"
        );
    }
}

/// Test 4a — the plugin fits beside the core's tool names.
#[test]
fn the_registry_accepts_the_plugin_beside_the_core_tools() {
    PluginRegistry::new(vec![Arc::new(chat_plugin())], &CORE_TOOLS)
        .expect("no core tool is a database tool");
}

/// Test 4b — a plugin tool the core already owns is a collision.
#[test]
fn the_registry_refuses_a_core_tool_the_plugin_declares() {
    let error = PluginRegistry::new(vec![Arc::new(chat_plugin())], &["bounded_sql_query"])
        .expect_err("the core owns bounded_sql_query");
    assert_eq!(
        error,
        PluginError::NameCollision {
            plugin: "database",
            tool: "bounded_sql_query".to_owned(),
        }
    );
}

/// Test 5 — an active chat plugin routes its own tool to its own executor and
/// routes nothing else at all.
#[test]
fn the_active_plugin_routes_its_declared_tools_only() {
    let plugin = Arc::new(chat_plugin());
    let executor = plugin.executor().expect("the chat plugin serves its tools");
    let registry =
        PluginRegistry::new(vec![plugin], &CORE_TOOLS).expect("no core tool is a database tool");
    let active = registry.active(&CHAT);

    let routed = active
        .route("schema_discovery")
        .expect("schema_discovery is the plugin's");
    assert!(
        Arc::ptr_eq(&routed, &executor),
        "the route is the plugin's own executor"
    );
    assert!(active.route("read").is_none(), "read is the core's");
}

/// Test 6 — the job adapter refuses a name it never declared rather than
/// forwarding it to the job's tools.
#[tokio::test]
async fn the_job_executor_refuses_a_name_it_never_declared() {
    let root = scratch("refuse");
    let plugin = DatabasePlugin::for_job(Arc::new(job_tools(&root, true)), None);
    let executor = plugin.executor().expect("the job plugin serves its tools");

    let error = executor
        .execute("read", serde_json::json!({"path": "anything"}))
        .await
        .expect_err("read is not a database tool");

    assert!(matches!(error, ToolError::UnsupportedTool), "{error}");
}
