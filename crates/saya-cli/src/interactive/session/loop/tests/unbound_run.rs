#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod unbound_run_tests {
    //! The run a session with no bound workspace composes: no folder, no
    //! workspace tool, whichever surface opens it.
    //!
    //! A session that bound nothing — the trust question answered `n`, a launch
    //! from the OS temporary directory — has no folder a run can work in. Its
    //! run is composed exactly as a session over a folder that cannot root an
    //! in-place job is: without a workspace, the database surface and nothing
    //! else. Never over the directory the process happens to stand in, whose
    //! shape would hand the run the file, shell and write tools the operator
    //! declined.
    //!
    //! The opposite is here too: a launch folder the operator answered `y` for
    //! is bound like `--workspace`, so the run over it keeps the folder's tools.

    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use saya_agent::{CancellationToken, ToolError, ToolExecutor};
    use saya_types::JobId;

    use crate::GlobalOptions;
    use crate::agent::tools::coding::RootTools;
    use crate::chat::Workspace;
    use crate::commands::coding::{
        CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob,
    };
    use crate::config::runtime::load_with_sources;
    use crate::interactive::session::line_chat::open_host;

    /// The session these rows acquire, and the id its run records under.
    pub(super) const ID: &str = "unbound-run";

    /// The tools a run with no workspace must never advertise: the read and
    /// search family, the write family, `rewind`, the shell lane and the
    /// helper lane.
    const WORKSPACE_TOOLS: &[&str] = &[
        "read",
        "glob",
        "grep",
        "list",
        "write",
        "edit",
        "delete",
        "notebook_edit",
        "rewind",
        "shell",
        "helpers",
    ];

    /// A tree this test owns, under the workspace's own `target/`: the folder
    /// the session is launched in, the config home it reads, its records.
    pub(super) fn scratch(label: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("the crate sits two levels under the workspace root")
            .join("target/tmp-unbound")
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock after epoch")
                    .as_nanos()
            ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("launch-dir")).expect("the launch dir is made");
        std::fs::create_dir_all(root.join("config")).expect("the config home is made");
        // The records' own root: the rule reads it when it decides whether the
        // folder can root a run, so it exists before the run is composed —
        // owner-private, as every state root the host opens over is.
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder
            .create(root.join("coding"))
            .expect("the records root is made");
        root
    }

    /// The launch's configuration over the row's own launch directory: no
    /// profile and no endpoint, so nothing real is composed.
    pub(super) fn runtime_over(root: &Path) -> RuntimeConfig {
        load_with_sources(
            &GlobalOptions::default(),
            &root.join("launch-dir"),
            &root.join("config"),
            BTreeMap::new(),
        )
        .expect("the row's runtime resolves")
    }

    /// A session that bound no root: a resume-shaped composition with no walk
    /// and no pinned root, which is what a launch that answered the trust
    /// question `n` — or one from the OS temporary directory — holds.
    fn unbound(root: &Path, runtime: &RuntimeConfig) -> SessionRuntime {
        let session = SessionRuntime::acquire(
            runtime,
            None,
            false,
            None,
            ID,
            saya_agent::ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the fixture session acquires");
        assert!(
            session.root().is_none(),
            "the fixture session bound no root: {:?}",
            session.root()
        );
        session
    }

    /// The run the parts open and the toolset it advertises, with its records
    /// under the row's own tree.
    pub(super) async fn run_over(mut parts: LineSession, root: &Path) -> (bool, RootTools) {
        parts.state_root = Some(root.join("coding"));
        let (application, shape) = open_host(&parts)
            .await
            .expect("the run's host opens over its parts");
        let has_workspace = application.has_workspace();
        // The write scope a session's run states is its folder's shape
        // (`chat::compose`): the folder itself when it roots one, nothing when
        // it does not.
        let scope = match shape.workspace() {
            Workspace::Here => vec![".".to_owned()],
            Workspace::None => Vec::new(),
        };
        let job = job_over(&application, "unbound", scope).await;
        let (tools, _) = RootTools::build(&job)
            .await
            .expect("the toolset composes");
        (has_workspace, tools)
    }

    /// One job over `application`, so the toolset the run advertises can be
    /// read.
    async fn job_over(
        application: &Arc<CodingApplication>,
        label: &str,
        scope: Vec<String>,
    ) -> Arc<NativeCodingJob> {
        let request = CodingJobRequest::new(
            JobId::parse(&format!("unbound-run-{label}")).expect("the label is a job id"),
            format!("session {label}"),
            scope,
        )
        .expect("the session's request is valid");
        let approval = CodingAuthorization::scoped(
            &application
                .propose(&request)
                .await
                .expect("the application proposes the request"),
        )
        .expect("the proposal is the authorization");
        application
            .create(request, &approval, CancellationToken::new())
            .await
            .expect("the session's job is created")
    }

    pub(super) fn advertises(definitions: &[saya_agent::ToolDefinition], name: &str) -> bool {
        definitions.iter().any(|definition| definition.name == name)
    }

    /// What a run with no workspace must never offer, and the call it must
    /// refuse.
    async fn assert_no_workspace(tools: &RootTools, surface: &str) {
        let advertised = tools.definitions();
        for name in WORKSPACE_TOOLS {
            assert!(
                !advertises(&advertised, name),
                "`{name}` must not be advertised by the {surface} of a session with \
                 no workspace: {advertised:?}"
            );
        }
        assert!(
            matches!(
                tools
                    .execute("read", serde_json::json!({"path": "README"}))
                    .await,
                Err(ToolError::UnsupportedTool)
            ),
            "the {surface} serves no file call without a workspace"
        );
    }

    #[tokio::test]
    async fn a_terminal_launch_on_an_unbound_session_composes_no_workspace() {
        let root = scratch("surface");
        let runtime = runtime_over(&root);
        let session = unbound(&root, &runtime);
        let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
        let state = SessionState::new(ID, None, "test-model");

        let composed = surface_parts(&state, &session, &runtime, &state_db, RenderFormat::Text);
        let (has_workspace, tools) = run_over(composed.parts, &root).await;
        assert!(
            !has_workspace,
            "a session that bound no workspace composes its run without one"
        );
        assert_no_workspace(&tools, "terminal launch").await;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn the_line_fronts_first_message_on_an_unbound_session_composes_no_workspace() {
        let root = scratch("first-message");
        let runtime = runtime_over(&root);
        let session = unbound(&root, &runtime);
        let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
        let state = SessionState::new(ID, None, "test-model");
        let tasks = session.universe().tasks();

        let parts = first_message_parts(
            &state,
            &session,
            Arc::new(runtime),
            &state_db,
            RenderFormat::Text,
            false,
            tasks,
        )
        .expect("the first message composes its run");
        let (has_workspace, tools) = run_over(parts, &root).await;
        assert!(
            !has_workspace,
            "a session that bound no workspace composes its first run without one"
        );
        assert_no_workspace(&tools, "line front's first message").await;
        let _ = std::fs::remove_dir_all(&root);
    }
}
