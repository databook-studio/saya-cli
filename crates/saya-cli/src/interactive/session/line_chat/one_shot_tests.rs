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
use crate::chat::{WorkspaceShape, workspace_shape};
use crate::cli::RootMode;
use crate::commands::coding::{CodingApplication, CodingAuthorization, CodingJobRequest};
use crate::config::runtime::load_with_sources;

use super::{Workspace, state_on, workspace};

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

/// One job over `folder`, opened the way the ask's own composition opens it and
/// with no write scope — the shape a one-turn ask states.
async fn ask_job(
    folder: &Path,
    label: &str,
    mode: RootMode,
) -> Arc<crate::commands::coding::NativeCodingJob> {
    let records = scratch(label);
    let runtime = load_with_sources(&GlobalOptions::default(), folder, &records, BTreeMap::new())
        .expect("the folder's runtime resolves");
    let store = SqliteStateStore::new(records.join("state.sqlite3"));
    let token = CancellationToken::new();
    let state_root = records.join("coding");
    let application = match mode {
        RootMode::NoWorkspace => {
            CodingApplication::session_without_workspace(
                runtime,
                store,
                folder,
                RootMode::NoWorkspace,
                &state_root,
                &token,
            )
            .await
        }
        other => CodingApplication::session(runtime, store, folder, other, &token).await,
    }
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
        workspace(&shape),
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
    // The same rule over a folder that can root a job: it reads the folder.
    assert_eq!(
        workspace(&workspace_shape(&plain, &plain)),
        Workspace::Here,
        "a plain folder roots the run"
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
}
