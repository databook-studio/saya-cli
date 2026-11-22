//! The one-turn ask's workspace rule, driven from the folder that cannot root
//! an in-place job: the home directory.
//!
//! `saya ask` asked from `$HOME` is a question, not a job: the folder cannot
//! root one, so the run takes no workspace at all — the database surface and
//! nothing else — instead of being refused. The opposite folder is here too, so
//! the test proves the rule reads the folder rather than always answering
//! "none".
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use saya_agent::{CancellationToken, ToolError, ToolExecutor};
use saya_store::SqliteStateStore;
use saya_types::JobId;

use crate::GlobalOptions;
use crate::agent::tools::coding::RootTools;
use crate::chat::{Workspace, WorkspaceShape, workspace_shape};
use crate::cli::RootMode;
use crate::commands::coding::{CodingApplication, CodingAuthorization, CodingJobRequest};
use crate::config::runtime::load_with_sources;

use super::state_on;

/// The workspace tools a run with no workspace must never offer: the read and
/// search family, the write family, the shell, and the helper lane.
const WORKSPACE_TOOLS: &[&str] = &[
    "read",
    "glob",
    "grep",
    "list",
    "write",
    "edit",
    "delete",
    "notebook_edit",
    "shell",
    "helpers",
];

/// A directory this test owns, for the records the runs below write.
fn scratch(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-ask-workspace-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

/// The configuration this row owns, stated through the loader's own parameters:
/// a config home of its own and its own project file.
///
/// Both are needed. The home the row asks from IS the folder the run is asked
/// in (`$HOME`), so the loader would otherwise read this machine's
/// `$HOME/.saya/config.toml` as the project layer — a file the row does not own
/// and cannot rely on: another checkout's retired key would fail the row before
/// it ever reached its subject. The home is `SAYA_CONFIG_HOME/saya`, the shape
/// the production loader is handed, and nothing here touches the process
/// environment.
fn own_config(records: &Path) -> (PathBuf, GlobalOptions) {
    let home = records.join("config").join("saya");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("config.toml"), "# this row's own config home\n").unwrap();
    let project = records.join("project.toml");
    fs::write(&project, "# this row's own project layer\n").unwrap();
    // The connections are stated too: discovery would look for them beside the
    // project layer, in the same home this row must not read.
    let connections = records.join("connections.toml");
    fs::write(&connections, "# this row connects to nothing\n").unwrap();
    let options = GlobalOptions {
        config: Some(project),
        connections: Some(connections),
        ..GlobalOptions::default()
    };
    (home, options)
}

/// One job over `folder`, opened the way the ask's own composition opens it and
/// with no write scope — the shape a one-turn ask states.
async fn ask_job(
    folder: &Path,
    label: &str,
    mode: RootMode,
) -> Arc<crate::commands::coding::NativeCodingJob> {
    let records = scratch(label);
    let (config_home, options) = own_config(&records);
    let runtime = load_with_sources(&options, folder, &config_home, BTreeMap::new())
        .expect("the folder's runtime resolves");
    let store = SqliteStateStore::new(records.join("state.sqlite3"));
    let token = CancellationToken::new();
    let state_root = records.join("coding");
    // The ask's own constructors, over a state root this test owns: the same
    // root the composition states when the configured one lands in the folder.
    let application =
        CodingApplication::session_over(runtime, store, folder, mode, &state_root, &token)
            .await
            .expect("the ask's application opens over the folder");
    // The one-turn facts the ask states on its host, from their one place.
    state_on(&application);
    let request = CodingJobRequest::new(
        JobId::parse(&format!("ask-workspace-{label}")).unwrap(),
        "count the rows".into(),
        Vec::new(),
    )
    .expect("a task with no write scope is a valid ask");
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .expect("the ask's job is created");
    let _ = fs::remove_dir_all(&records);
    job
}

fn advertises(definitions: &[saya_agent::ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|definition| definition.name == name)
}

#[tokio::test]
async fn ask_from_home_advertises_no_workspace_tool() {
    let home = std::env::var_os("HOME").expect("the test host states a home directory");
    let home = fs::canonicalize(home).expect("the home directory resolves");
    let records = scratch("home-root");

    // The shape the ask's own composition reads: $HOME cannot root a job.
    let shape = workspace_shape(&home, &records);
    assert!(
        matches!(shape, WorkspaceShape::NoWorkspace { .. }),
        "the home directory cannot root an in-place job: {shape:?}"
    );
    assert_eq!(
        shape.workspace(),
        Workspace::None,
        "the question is asked there without a workspace"
    );

    let job = ask_job(&home, "home", RootMode::NoWorkspace).await;
    assert!(!job.has_workspace(), "an ask from $HOME has no workspace");
    let (tools, _) = RootTools::build(&job).await.expect("the toolset composes");
    let advertised = tools.definitions();
    for name in WORKSPACE_TOOLS {
        assert!(
            !advertises(&advertised, name),
            "`{name}` must not be advertised by an ask with no workspace: {advertised:?}"
        );
    }
    assert!(
        matches!(
            tools
                .execute("read", serde_json::json!({"path": "README"}))
                .await,
            Err(ToolError::UnsupportedTool)
        ),
        "a run with no workspace serves no file call"
    );
    let _ = fs::remove_dir_all(&records);
}

#[tokio::test]
async fn ask_from_a_plain_folder_keeps_its_workspace() {
    let plain = scratch("plain-root");
    let records = scratch("plain-state");
    // The same rule over a folder that can root a job: it reads the folder. A
    // session's records live elsewhere, never inside it.
    let plain_shape = workspace_shape(&plain, &records);
    assert_eq!(
        plain_shape.workspace(),
        Workspace::Here,
        "a plain folder roots the run: {plain_shape:?}"
    );
    let job = ask_job(&plain, "plain", RootMode::InPlace).await;
    assert!(job.has_workspace(), "a plain folder is the run's workspace");
    let (tools, _) = RootTools::build(&job).await.expect("the toolset composes");
    let advertised = tools.definitions();
    for name in ["read", "glob", "grep"] {
        assert!(
            advertises(&advertised, name),
            "`{name}` is advertised over a plain folder: {advertised:?}"
        );
    }
    let _ = fs::remove_dir_all(&plain);
    let _ = fs::remove_dir_all(&records);
}
