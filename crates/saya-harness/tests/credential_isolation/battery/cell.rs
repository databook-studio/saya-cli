use super::helper::*;
use super::*;

// -- one cell: one run directory, proven sandbox, store, generated config

pub(super) struct Cell {
    pub(super) root: PathBuf,
    pub(super) run: RunDir,
    pub(super) run_id: RunId,
    pub(super) store: Arc<SqliteStateStore>,
    pub(super) provision: SandboxProvision,
}

impl Cell {
    /// A runner tool over the proven spawn, carrying exactly the
    /// credentials `credentials` lists — the composition root's per-step
    /// shape.
    pub(super) fn tool(
        &self,
        credentials: Vec<Credential>,
        source: Arc<StaticCredentialSource>,
    ) -> RunProgram {
        let scope = RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped");
        RunProgram::new(
            self.provision.spawn().expect("proven").clone(),
            scope,
            DEFAULT_TIMEOUT,
            source,
        )
        .with_credentials(credentials)
    }
}

/// A canonical per-test root: everything the cell owns lives under it,
/// and one cleanup covers all of it.
pub(super) fn cell_root(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "saya-credential-e2e-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("battery root must be creatable");
    fs::canonicalize(&path).expect("battery root must canonicalise")
}

pub(super) fn orchestrator_endpoint() -> EndpointSpec {
    EndpointSpec {
        name: "primary".to_owned(),
        provider: "anthropic".to_owned(),
        model: "mock-model".to_owned(),
        base_url: Some("https://api.test".to_owned()),
        api_key: Some(SecretRef::Env {
            env: ORCHESTRATOR_SOURCE.to_owned(),
        }),
    }
}

pub(super) fn reviewer_endpoint() -> EndpointSpec {
    EndpointSpec {
        name: "reviewer-ep".to_owned(),
        provider: "anthropic".to_owned(),
        model: "mock-model".to_owned(),
        base_url: Some("https://api.test".to_owned()),
        api_key: Some(SecretRef::Env {
            env: REVIEWER_SOURCE.to_owned(),
        }),
    }
}

pub(super) fn orchestrator_credential() -> Credential {
    Credential::new(
        "orchestrator-key",
        endpoint_env_var(ORCHESTRATOR_ROLE),
        SecretRef::Env {
            env: ORCHESTRATOR_SOURCE.to_owned(),
        },
    )
    .expect("the declared credential is well-shaped")
}

pub(super) fn reviewer_credential() -> Credential {
    Credential::new(
        "reviewer-key",
        endpoint_env_var("reviewer"),
        SecretRef::Env {
            env: REVIEWER_SOURCE.to_owned(),
        },
    )
    .expect("the declared credential is well-shaped")
}

pub(super) fn source_with(pairs: &[(&str, &str)]) -> Arc<StaticCredentialSource> {
    Arc::new(StaticCredentialSource::new(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    ))
}

/// Builds one cell: the run directory (`runs/<id>/` with its workspace
/// and state), the generated config over the given endpoint pool, the
/// store with the run standing at `Approved` (the state a run is in when
/// its first episode begins), the staged helper outside the fs roots,
/// and the proven sandbox over the run's own workspace — so the runner's
/// disk records land inside the run directory the sweep walks.
pub(super) async fn cell(
    label: &str,
    endpoints: &[EndpointSpec],
    bindings: &EndpointBindings,
) -> Cell {
    let root = cell_root(label);
    let run_id = RunId::parse(&format!("e2e-{label}")).expect("shaped run id");
    let run = RunDir::create(&root.join("runs"), &run_id).expect("the run directory creates");

    let journal = Journal::open(run.root());
    journal
        .append(&RunEvent::RunStarted)
        .expect("the journal appends");
    journal
        .append(&RunEvent::PlanApproved { scopes: None })
        .expect("the journal appends");

    let store = Arc::new(SqliteStateStore::new(root.join("state.sqlite3")));
    RunStore::create_run(
        &*store,
        NewRun {
            id: run_id.clone(),
            capabilities: RunCapabilityFlags::default(),
            budgets: RunBudgets::default(),
        },
    )
    .await
    .expect("the store creates the run");
    RunStore::set_run_status(&*store, &run_id, RunStatus::Approved, None)
        .await
        .expect("the run stands approved");

    write_run_configs(
        &run,
        endpoints,
        bindings,
        &[CorpusProfile {
            name: "corp".to_owned(),
            path: "workspace/corpus/corp.db".to_owned(),
        }],
    )
    .expect("the generated configs write");

    let programs = root.join("programs");
    fs::create_dir_all(&programs).expect("the program directory stages");
    fs::copy(helper(), programs.join(PROGRAM)).expect("the helper must stage");

    let policy = RunSandbox::new([run.workspace().to_path_buf()], Vec::<(String, u16)>::new())
        .expect("the workspace root constructs the policy");
    let provision = policy.prepare(&programs).expect("preparation runs");
    assert!(
        provision.report().proves_runner(),
        "the healthy-host probe must prove the battery:\n{}",
        provision.report().render()
    );
    Cell {
        root,
        run,
        run_id,
        store,
        provision,
    }
}
