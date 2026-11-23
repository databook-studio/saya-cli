//! The line session's workspace rule, driven from the two folder shapes a
//! headless session can be started over.
//!
//! A session whose folder cannot root an in-place job — the home directory, a
//! dangerous root, a folder that would contain the job's private state root —
//! runs without a workspace: the database surface and nothing else, instead of
//! being refused before its first turn. The opposite folder is here too, so the
//! test proves the rule reads the folder rather than always answering "none".
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use saya_agent::{ApprovalPolicy, CancellationToken, SessionGrants, ToolError, ToolExecutor};
use saya_store::SqliteStateStore;
use saya_types::JobId;

use crate::GlobalOptions;
use crate::agent::tools::coding::RootTools;
use crate::agent::turn_config::PromptOverrides;
use crate::chat::{Workspace, WorkspaceShape};
use crate::commands::coding::{
    CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob,
};
use crate::config::runtime::load_with_sources;
use crate::plugins::database::DatabaseBudget;
use crate::render::RenderFormat;

use super::open::LineSession;

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

/// A directory this test owns, for the folder a session opens over and for the
/// records its runs write.
fn scratch(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-session-workspace-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

/// The session's records directory, owner-private: the rule the host applies to
/// every state root it opens over.
fn private_dir(path: &Path) {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).unwrap();
}

/// The session facts the line loop states on its first message, over `folder`
/// and under the state root it names: `None` takes the configured root, a path
/// states one.
fn session(folder: &Path, records: &Path, state_root: Option<PathBuf>) -> LineSession {
    let runtime = load_with_sources(&GlobalOptions::default(), folder, records, BTreeMap::new())
        .expect("the folder's runtime resolves");
    LineSession {
        session: "workspace-rule".into(),
        folder: folder.to_path_buf(),
        runtime: Arc::new(runtime),
        store: SqliteStateStore::new(records.join("state.sqlite3")),
        mode: ApprovalPolicy::ReadOnly,
        overrides: PromptOverrides::default(),
        format: RenderFormat::Text,
        sink: None,
        budget: DatabaseBudget::defaults(),
        can_prompt: false,
        grants: SessionGrants::default(),
        state_root,
        cancellation: None,
        tasks: None,
        facts: None,
        seed: Vec::new(),
    }
}

/// The host the session composes its run over: the composition the line loop's
/// first message runs, at the root it opens over.
async fn open(parts: &LineSession) -> Arc<CodingApplication> {
    let (application, shape) = super::session_host::open_host(parts)
        .await
        .expect("a session opens over the folder it was started in");
    assert_eq!(
        shape.workspace() == Workspace::None,
        !application.has_workspace(),
        "the host states the workspace the folder's shape gives it"
    );
    application
}

/// One read-only job over `application`, so the toolset the session's run
/// advertises can be read.
async fn job(application: &Arc<CodingApplication>, label: &str) -> Arc<NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse(&format!("session-workspace-{label}")).unwrap(),
        format!("session {label}"),
        Vec::new(),
    )
    .expect("a task with no write scope is a valid session turn");
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    application
        .create(request, &approval, CancellationToken::new())
        .await
        .expect("the session's job is created")
}

fn advertises(definitions: &[saya_agent::ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|definition| definition.name == name)
}

#[tokio::test]
async fn a_session_over_a_folder_that_holds_its_state_root_advertises_no_workspace_tool() {
    let folder = scratch("holding");
    let records = scratch("holding-state");
    // The shape that cannot root an in-place job here: the folder contains the
    // root the session's job records under.
    let state_root = folder.join("coding");
    private_dir(&state_root);
    let parts = session(&folder, &records, Some(state_root));
    assert!(
        matches!(
            crate::chat::workspace_shape(
                &parts.folder,
                parts.state_root.as_deref().unwrap(),
                &parts.session,
            ),
            WorkspaceShape::NoWorkspace { .. }
        ),
        "the folder that holds the job's private state root cannot root it"
    );

    let application = open(&parts).await;
    assert!(
        !application.has_workspace(),
        "a session over a folder that holds its state root has no workspace"
    );
    let job = job(&application, "holding").await;
    assert!(!job.has_workspace(), "its run has no workspace either");
    let (tools, _) = RootTools::build(&job).await.expect("the toolset composes");
    let advertised = tools.definitions();
    for name in WORKSPACE_TOOLS {
        assert!(
            !advertises(&advertised, name),
            "`{name}` must not be advertised by a session with no workspace: {advertised:?}"
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
    let _ = fs::remove_dir_all(&folder);
    let _ = fs::remove_dir_all(&records);
}

#[tokio::test]
async fn a_session_over_a_bound_workspace_keeps_its_workspace_tools() {
    let folder = scratch("bound");
    let records = scratch("bound-state");
    // The same rule over a folder that can root a job: a session bound to it
    // records under a state root outside it, so it keeps its files.
    let state_root = records.join("coding");
    private_dir(&state_root);
    let parts = session(&folder, &records, Some(state_root));
    assert_eq!(
        crate::chat::workspace_shape(
            &parts.folder,
            parts.state_root.as_deref().unwrap(),
            &parts.session,
        ),
        WorkspaceShape::Writable,
        "a bound folder roots the run"
    );

    let application = open(&parts).await;
    assert!(
        application.has_workspace(),
        "a session with a bound workspace has one"
    );
    let job = job(&application, "bound").await;
    assert!(job.has_workspace(), "its run has the workspace too");
    let (tools, _) = RootTools::build(&job).await.expect("the toolset composes");
    let advertised = tools.definitions();
    for name in ["read", "glob", "grep"] {
        assert!(
            advertises(&advertised, name),
            "`{name}` is advertised over a bound workspace: {advertised:?}"
        );
    }
    let _ = fs::remove_dir_all(&folder);
    let _ = fs::remove_dir_all(&records);
}
