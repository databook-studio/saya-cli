//! The M5-7 credential-isolation end-to-end battery: the planted-secret
//! tests, not the spawn. The claim the design makes is narrow and is tested
//! here rather than asserted: **a credential reaches a child only when it
//! was declared, the child is sandboxed, the generated config holds
//! references only, and captured output is redacted.** Remove any one
//! condition and the credential must not appear.
//!
//! What this battery adds over the M5-4 escape battery (`tests/runner.rs`),
//! which owns the four-condition rule at the spawn level and is read, not
//! duplicated:
//!
//! 1. **The full sweep.** A resolved credential must appear in exactly one
//!    place — the child's environment, proven by a digest the child computes
//!    of its own env value (the raw value is never printed in this test, so
//!    the sweep's only expected hit is the environment itself). The whole
//!    run directory is scanned recursively — generated configs, the journal,
//!    the runner's disk records — plus the store database with its WAL
//!    sidecars, plus the tool result the model actually received. Any other
//!    hit fails.
//! 2. **The exfiltration shape.** A child that echoes its own environment —
//!    `SAYA_RUN_EP_ORCHESTRATOR=<value>`, under the very name the generated
//!    config binds — is the credential leaving through captured output. The
//!    model lane (the `tool` message the loop hands the provider) and the
//!    disk lane (the record the runner persists) are asserted separately;
//!    they are different code paths. This test found a real hole: pattern
//!    redaction alone let the exact injected value through, and the capture
//!    boundary now scrubs the resolved values themselves (see the M5-7
//!    report).
//! 3. **The four conditions end to end** — through the episode loop, the
//!    engine sink, the journal, and the store, not only at the spawn call.
//!    "Declared" and "references-only" are re-proven here; "sandboxed" is
//!    structural and has no end-to-end variant to cover (no proven
//!    `RunnerSpawn`, no tool, no credential list to attach — the escape
//!    battery's construction refusal is the whole of that proof); "redact"
//!    is test 2.
//! 4. **Role separation.** A reviewer credential is absent from an
//!    orchestrator child's environment and the other way round, within one
//!    run driven step by step, with the generated config binding each role
//!    to its own run-scoped env variable.
//! 5. **The failure path.** A run that fails mid-flight (the provider fails
//!    after the child ran; the bounded retry spends itself; the run pauses)
//!    leaves a journal and a store whose bytes carry no credential material.
//!
//! Unproven here, and said so rather than tested vacuously: the nested-saya
//! leg — the composition root does not yet attach the runner tool to a run's
//! toolset, so no nested `saya` child holds a runner credential and there is
//! no path to exercise (see the M5-7 report). The startup probe proves the
//! sandbox on macOS only; elsewhere the runner is absent and these tests
//! compile to nothing rather than pass vacuously.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod battery {
    use std::{
        collections::VecDeque,
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::{Arc, Mutex, OnceLock},
        time::Duration,
    };

    use async_trait::async_trait;
    use saya_agent::{
        ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
        ProviderError, ToolCall, ToolDefinition, ToolExecutor,
    };
    use saya_harness::endpoints::{
        CorpusProfile, EndpointSpec, ORCHESTRATOR_ROLE, endpoint_env_var, write_run_configs,
    };
    use saya_harness::engine::{
        EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeError, EpisodeRequest,
        EpisodeRun, ManifestBounds, RunState, SinkBudgets, StepToolset, UsageTotals,
    };
    use saya_harness::journal::{EVENTS_FILE, Journal};
    use saya_harness::run_dir::RunDir;
    use saya_harness::runner::sandbox::{RunSandbox, SandboxProvision};
    use saya_harness::runner::{Credential, RUN_PROGRAM_TOOL, RunProgram, StaticCredentialSource};
    use saya_harness::workspace::Workspace;
    use saya_store::{
        NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStepStatus, RunStore,
        SqliteStateStore,
    };
    use saya_types::{
        Capabilities, EndpointBindings, PauseReason, RunEvent, RunId, RunPlan, RunnerScope,
        SecretRef, StepSpec,
    };

    /// The allowlisted runner program: the battery's compiled helper.
    const PROGRAM: &str = "saya-probe";
    /// The tool's configured default timeout in the battery.
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

    /// The runner-side reference source for the orchestrator's credential —
    /// what a production resolver would read from the user's environment.
    const ORCHESTRATOR_SOURCE: &str = "SAYA_E2E_ORCHESTRATOR_SOURCE";
    /// The planted orchestrator credential: distinctive, searchable, and
    /// deliberately carrying **no** redaction marker (`password=`, `api_key=`,
    /// a credential header, URL userinfo) — a bare value must be scrubbed even
    /// when the only thing naming it is the env variable it was injected
    /// under.
    const ORCHESTRATOR_VALUE: &str = "planted-orchestrator-7Qk2mVn9xR4e2e";
    /// The reviewer role's source and planted value (role separation only).
    const REVIEWER_SOURCE: &str = "SAYA_E2E_REVIEWER_SOURCE";
    const REVIEWER_VALUE: &str = "planted-reviewer-Zx4WpHj3Le2e2";

    // -- the compiled helper -------------------------------------------------

    /// The battery's helper: a digest dumper (FNV-1a 64 of one env value —
    /// proves the child's environment held exactly the planted value without
    /// the value ever being printed), a name-only presence prober, and an
    /// exfiltration mode that echoes the recognised secret shapes plus the
    /// credential under its real env name. One binary, compiled with the
    /// toolchain running the tests.
    const HELPER_SOURCE: &str = r#"
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    match mode {
        "digest" => {
            let var = args.get(2).cloned().unwrap_or_default();
            match std::env::var(&var) {
                Ok(value) => println!("DIGEST {} {:016x}", var, fnv1a(value.as_bytes())),
                Err(_) => println!("DIGEST {} UNSET", var),
            }
        }
        "which" => {
            for var in args.iter().skip(2) {
                match std::env::var(var) {
                    Ok(_) => println!("SET {}", var),
                    Err(_) => println!("UNSET {}", var),
                }
            }
        }
        "leak" => {
            let var = args.get(2).cloned().unwrap_or_default();
            println!("password=hunter2-marker-shape");
            println!("Authorization: Bearer sk-live-shape-marker");
            println!("https://user:hunter2-url-marker@db.example.com/x");
            println!("{}={}", var, std::env::var(&var).unwrap_or_default());
        }
        _ => {
            eprintln!("unknown mode");
            std::process::exit(2);
        }
    }
}
"#;

    fn helper() -> &'static PathBuf {
        static HELPER: OnceLock<PathBuf> = OnceLock::new();
        HELPER.get_or_init(|| {
            let dir = std::env::var("CARGO_TARGET_TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            let source = dir.join("saya-credential-helper.rs");
            let binary = dir.join("saya-credential-helper");
            fs::write(&source, HELPER_SOURCE).expect("helper source must be written");
            let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
            let status = Command::new(rustc)
                .arg("--edition=2021")
                .arg("-o")
                .arg(&binary)
                .arg(&source)
                .status()
                .expect("rustc must be runnable — it is the toolchain running this test");
            assert!(
                status.success(),
                "the battery helper must compile: {status}"
            );
            binary
        })
    }

    /// The digest the helper computes of an env value; test and helper must
    /// agree, so the child's own report of its environment is the assertion.
    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    // -- one cell: one run directory, proven sandbox, store, generated config

    struct Cell {
        root: PathBuf,
        run: RunDir,
        run_id: RunId,
        store: Arc<SqliteStateStore>,
        provision: SandboxProvision,
    }

    impl Cell {
        /// A runner tool over the proven spawn, carrying exactly the
        /// credentials `credentials` lists — the composition root's per-step
        /// shape.
        fn tool(
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
    fn cell_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "saya-credential-e2e-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("battery root must be creatable");
        fs::canonicalize(&path).expect("battery root must canonicalise")
    }

    fn orchestrator_endpoint() -> EndpointSpec {
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

    fn reviewer_endpoint() -> EndpointSpec {
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

    fn orchestrator_credential() -> Credential {
        Credential::new(
            "orchestrator-key",
            endpoint_env_var(ORCHESTRATOR_ROLE),
            SecretRef::Env {
                env: ORCHESTRATOR_SOURCE.to_owned(),
            },
        )
        .expect("the declared credential is well-shaped")
    }

    fn reviewer_credential() -> Credential {
        Credential::new(
            "reviewer-key",
            endpoint_env_var("reviewer"),
            SecretRef::Env {
                env: REVIEWER_SOURCE.to_owned(),
            },
        )
        .expect("the declared credential is well-shaped")
    }

    fn source_with(pairs: &[(&str, &str)]) -> Arc<StaticCredentialSource> {
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
    async fn cell(label: &str, endpoints: &[EndpointSpec], bindings: &EndpointBindings) -> Cell {
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

    // -- the scripted provider, the approval stub, and the engine drive -------

    /// A scripted provider turn.
    enum Turn {
        /// An assistant turn carrying tool calls.
        Tools(Vec<ToolCall>),
        /// A terminal prose answer (no tool calls).
        Answer(&'static str),
        /// A provider failure.
        Fail,
    }

    /// A scripted `ChatProvider`: serves its turns in order and records every
    /// request it received — the model lane is read from what it received.
    struct ScriptProvider {
        script: Mutex<VecDeque<Turn>>,
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }

    impl ScriptProvider {
        fn new(script: Vec<Turn>) -> Self {
            Self {
                script: Mutex::new(script.into()),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn requests(&self) -> Vec<ChatRequest> {
            self.requests.lock().expect("provider request lock").clone()
        }
    }

    #[async_trait]
    impl ChatProvider for ScriptProvider {
        fn name(&self) -> &str {
            "script"
        }

        async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.requests
                .lock()
                .expect("provider request lock")
                .push(request);
            match self
                .script
                .lock()
                .expect("provider script lock")
                .pop_front()
            {
                Some(Turn::Answer(text)) => {
                    Ok(ChatResponse::new(ChatMessage::text("assistant", text)))
                }
                Some(Turn::Tools(calls)) => Ok(ChatResponse::new(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: calls,
                    tool_call_id: None,
                })),
                Some(Turn::Fail) => Err(ProviderError::Request("scripted failure".into())),
                None => panic!("provider script exhausted"),
            }
        }
    }

    struct AllowApproval;

    #[async_trait]
    impl ApprovalDecider for AllowApproval {
        async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
            true
        }
    }

    fn bounds() -> ManifestBounds {
        ManifestBounds {
            max_files: 512,
            max_file_bytes: 256 * 1024,
        }
    }

    /// A step carrying the runner capability: the driver narrows the run's
    /// tool universe to it, and the loop's workspace-write gate is satisfied
    /// by the capability the step declares.
    fn runner_step(goal: &str) -> StepSpec {
        let mut capabilities = Capabilities::default();
        capabilities.workspace_write = true;
        capabilities.runner = Some(RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"));
        StepSpec::new(goal, capabilities, None, Vec::new(), None).expect("well-shaped")
    }

    fn sink(cell: &Cell) -> EngineEventSink {
        EngineEventSink::new(
            cell.run_id.clone(),
            RunState::Approved,
            Journal::open(cell.run.root()),
            cell.store.clone(),
            SinkBudgets {
                wall_clock: None,
                token_ceiling: None,
                download_budget: None,
                carried_usage: UsageTotals::default(),
            },
            std::time::Instant::now,
        )
    }

    /// Drives one plan step end to end: brief, narrowed tools, the loop, the
    /// tool result back to the provider, the sink's journal and store
    /// mirrors. The driver is per-step, exactly like the composition root's.
    async fn drive_step(
        cell: &Cell,
        sink: &EngineEventSink,
        tool: RunProgram,
        provider: &ScriptProvider,
        plan: &RunPlan,
        step: usize,
    ) -> Result<(), EpisodeError> {
        let approval = AllowApproval;
        let definition = tool.definition();
        let executor: Arc<dyn ToolExecutor> = Arc::new(tool);
        // One toolset per plan step over the same executor, the way the
        // composition root builds them; only the driven step's episode runs.
        let toolsets: Vec<StepToolset> = (0..plan.steps.len())
            .map(|_| StepToolset {
                executor: Arc::clone(&executor),
                definitions: vec![definition.clone()],
            })
            .collect();
        let driver = EpisodeDriver::new(
            EpisodeCollaborators {
                provider,
                approval: &approval,
                toolsets: &toolsets,
                cancellation: CancellationToken::default(),
            },
            EpisodeRun {
                run_id: cell.run_id.clone(),
                store: cell.store.clone(),
                journal: Journal::open(cell.run.root()),
            },
            EpisodeRequest {
                model: "mock-model".into(),
                profile_names: Vec::new(),
                memory_allows_candidate_writes: false,
            },
            bounds(),
        );
        let workspace = Workspace::open(cell.run.workspace()).expect("the run workspace opens");
        driver.run_step(sink, plan, step, &workspace).await
    }

    fn run_program_call(args: &[&str]) -> ToolCall {
        ToolCall {
            id: "c-run".to_owned(),
            name: RUN_PROGRAM_TOOL.to_owned(),
            arguments: serde_json::json!({
                "program": PROGRAM,
                "args": args,
                "timeout_seconds": 30
            }),
        }
    }

    // -- reading the lanes the capture reaches --------------------------------

    /// Every `tool`-role message the provider received, joined — the
    /// model-facing copy of the tool result.
    fn tool_lane(requests: &[ChatRequest]) -> String {
        requests
            .iter()
            .flat_map(|request| request.messages.iter())
            .filter(|message| message.role == "tool")
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every regular file under `root`, at any depth.
    fn files_under(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files
    }

    /// Byte-scans every file for the needle; any hit fails with the file
    /// named. The scan is exact-window, so a near-miss is not a hit.
    fn assert_absent(files: &[PathBuf], needle: &str, lane: &str) {
        for file in files {
            let bytes = fs::read(file).unwrap_or_default();
            assert!(
                !bytes
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes()),
                "{lane}: the planted credential reached {}: {file:?}",
                file.display()
            );
        }
    }

    /// Byte-scans raw bytes (the store database and its sidecars, the tool
    /// lane text) for the needle.
    fn assert_absent_bytes(bytes: &[u8], needle: &str, lane: &str) {
        assert!(
            !bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes()),
            "{lane}: the planted credential appears in the bytes"
        );
    }

    /// The store database's raw bytes plus any populated `-wal`/`-shm`
    /// sidecars — a freshly written row may live only in the WAL, so
    /// scanning all three is what makes the check honest.
    fn store_bytes(cell: &Cell) -> Vec<u8> {
        let db = cell.root.join("state.sqlite3");
        let mut out =
            fs::read(&db).unwrap_or_else(|error| panic!("the store database must exist: {error}"));
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", db.display()));
            if sidecar.exists() {
                out.extend(fs::read(&sidecar).unwrap_or_default());
            }
        }
        out
    }

    /// The runner's disk records: the files under the workspace's
    /// `run_program/` directory.
    fn record_files(cell: &Cell) -> Vec<PathBuf> {
        files_under(cell.run.root())
            .into_iter()
            .filter(|path| {
                path.components()
                    .any(|component| component.as_os_str() == "run_program")
            })
            .collect()
    }

    /// Every place the planted value is forbidden, swept: the whole run
    /// directory (generated configs, journal, disk records, workspace), the
    /// store database with its sidecars, and the journal's parsed events.
    fn sweep_run(cell: &Cell, value: &str) {
        assert_absent(&files_under(cell.run.root()), value, "the run directory");
        let store = store_bytes(cell);
        assert_absent_bytes(&store, value, "the store database");
        let journal = fs::read_to_string(cell.run.root().join(EVENTS_FILE))
            .expect("the journal must be readable");
        assert!(
            !journal.contains(value),
            "the journal carries credential material: {journal}"
        );
        let events = Journal::open(cell.run.root())
            .read()
            .expect("the journal parses");
        assert!(
            !format!("{events:?}").contains(value),
            "the journal's parsed events carry credential material"
        );
    }

    fn cleanup(cell: &Cell) {
        let _ = fs::remove_dir_all(&cell.root);
    }

    // -- 1: the planted secret, full sweep ------------------------------------

    /// A resolved credential must appear in exactly one place: the child's
    /// environment. The child reports a digest of its own env value — the
    /// test compares it against the digest of the planted value, proving the
    /// environment held exactly that value — and everything else is swept:
    /// the whole run directory recursively, the journal (bytes and parsed
    /// events), the store database with its WAL sidecars, the disk records,
    /// and the tool result the model received. The generated config is
    /// asserted to hold the reference only.
    #[tokio::test]
    async fn a_resolved_credential_lives_only_in_the_child_s_environment() {
        let cell = cell(
            "sweep",
            &[orchestrator_endpoint()],
            &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
        )
        .await;
        let var = endpoint_env_var(ORCHESTRATOR_ROLE);
        let provider = ScriptProvider::new(vec![
            Turn::Tools(vec![run_program_call(&["digest", &var])]),
            Turn::Answer("done"),
        ]);
        let tool = cell.tool(
            vec![orchestrator_credential()],
            source_with(&[(ORCHESTRATOR_SOURCE, ORCHESTRATOR_VALUE)]),
        );
        let plan = RunPlan::new(vec![runner_step("measure the child's environment")])
            .expect("well-shaped plan");
        let sink = sink(&cell);
        drive_step(&cell, &sink, tool, &provider, &plan, 0)
            .await
            .expect("the step completes");
        drop(sink);

        // The child's environment held exactly the planted credential — and
        // the raw value rode no captured byte.
        let model_lane = tool_lane(&provider.requests());
        let expected_digest = format!("DIGEST {var} {:016x}", fnv1a(ORCHESTRATOR_VALUE.as_bytes()));
        assert!(
            model_lane.contains(&expected_digest),
            "the child's environment must have held exactly the planted value (digest match) \
             — tool result: {model_lane}"
        );
        assert!(
            !model_lane.contains(ORCHESTRATOR_VALUE),
            "the raw value must not ride the tool result: {model_lane}"
        );

        // Exactly one place: everything else is swept.
        sweep_run(&cell, ORCHESTRATOR_VALUE);
        assert_absent(&record_files(&cell), ORCHESTRATOR_VALUE, "the disk record");

        // Non-vacuity: the child really ran and its record landed, the store
        // really recorded the completed run.
        assert!(
            !record_files(&cell).is_empty(),
            "the disk record must exist for the sweep to be honest"
        );
        let run = RunStore::get_run(&*cell.store, &cell.run_id)
            .await
            .expect("the store reads")
            .expect("the run row exists");
        assert_eq!(run.status, RunStatus::Completed, "the run completed");

        // The generated config holds the reference, never a resolved value.
        let config = fs::read_to_string(cell.run.state().join("config/config.toml"))
            .expect("the generated config reads");
        assert!(
            config.contains(&format!("api_key = {{ env = \"{var}\" }}")),
            "the config binds the run-scoped env reference: {config}"
        );
        assert!(
            !config.contains(ORCHESTRATOR_SOURCE),
            "the original reference must not be echoed into the config: {config}"
        );
        assert!(
            !config.contains(ORCHESTRATOR_VALUE),
            "the resolved value must not reach the config: {config}"
        );

        cleanup(&cell);
    }

    // -- 2: a secret in captured stdout is redacted to the model and to disk --

    /// A child that echoes the credential it legitimately received — under
    /// the env name the generated config binds, next to the recognised
    /// secret shapes — is the credential leaving through captured output.
    /// The model lane (the `tool` message the loop hands the provider) and
    /// the disk lane (the record the runner persists into the run
    /// workspace) are asserted separately: different code paths, and one has
    /// been wrong before. This test exposed the hole the capture-boundary
    /// fix closes: pattern redaction alone cannot recognise a bare value.
    #[tokio::test]
    async fn a_secret_in_captured_stdout_is_redacted_before_the_model_and_before_disk() {
        let cell = cell(
            "exfiltration",
            &[orchestrator_endpoint()],
            &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
        )
        .await;
        let var = endpoint_env_var(ORCHESTRATOR_ROLE);
        let provider = ScriptProvider::new(vec![
            Turn::Tools(vec![run_program_call(&["leak", &var])]),
            Turn::Answer("done"),
        ]);
        let tool = cell.tool(
            vec![orchestrator_credential()],
            source_with(&[(ORCHESTRATOR_SOURCE, ORCHESTRATOR_VALUE)]),
        );
        let plan = RunPlan::new(vec![runner_step("echo the environment")]).expect("well-shaped");
        let sink = sink(&cell);
        drive_step(&cell, &sink, tool, &provider, &plan, 0)
            .await
            .expect("the step completes");
        drop(sink);

        let forbidden = [
            ORCHESTRATOR_VALUE,
            "hunter2-marker-shape",
            "sk-live-shape-marker",
            "hunter2-url-marker",
        ];
        // The model lane: the tool message the provider actually received.
        let model_lane = tool_lane(&provider.requests());
        assert!(
            model_lane.contains("SAYA_RUN_EP_ORCHESTRATOR=[redacted]"),
            "the env-name shape must survive with its value scrubbed — this is what proves \
             the child really echoed the credential: {model_lane}"
        );
        for secret in &forbidden {
            assert!(
                !model_lane.contains(secret),
                "the model lane carries a planted secret: {model_lane}"
            );
        }
        assert!(
            model_lane.contains("[redacted]"),
            "the model lane must show the redaction happened: {model_lane}"
        );

        // The disk lane: the record files the runner persisted, read from
        // disk — not from memory.
        let records = record_files(&cell);
        assert!(
            !records.is_empty(),
            "the runner's disk records must exist for the disk lane to be honest"
        );
        let disk_lane = records
            .iter()
            .map(|path| fs::read_to_string(path).expect("the record reads"))
            .collect::<Vec<_>>()
            .join("\n");
        for secret in &forbidden {
            assert!(
                !disk_lane.contains(secret),
                "the disk lane carries a planted secret"
            );
        }
        assert!(
            disk_lane.contains("[redacted]"),
            "the disk lane must show the redaction happened"
        );

        // The failure of either lane would also show in the full sweep.
        sweep_run(&cell, ORCHESTRATOR_VALUE);
        for secret in [
            "hunter2-marker-shape",
            "sk-live-shape-marker",
            "hunter2-url-marker",
        ] {
            sweep_run(&cell, secret);
        }

        cleanup(&cell);
    }

    // -- 3: the four conditions, end to end -----------------------------------

    /// Condition "declared" removed: the step's credential list is empty, so
    /// the variable never reaches the child's environment — proven through
    /// the episode loop, not at the spawn call (the escape battery's
    /// `an_undeclared_credential_never_reaches_the_child` owns that shape).
    #[tokio::test]
    async fn an_undeclared_credential_never_reaches_a_child_through_the_episode_loop() {
        let cell = cell(
            "undeclared",
            &[orchestrator_endpoint()],
            &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
        )
        .await;
        let var = endpoint_env_var(ORCHESTRATOR_ROLE);
        let provider = ScriptProvider::new(vec![
            Turn::Tools(vec![run_program_call(&["which", &var])]),
            Turn::Answer("done"),
        ]);
        let tool = cell.tool(Vec::new(), source_with(&[]));
        let plan = RunPlan::new(vec![runner_step("probe the environment")]).expect("well-shaped");
        let sink = sink(&cell);
        drive_step(&cell, &sink, tool, &provider, &plan, 0)
            .await
            .expect("the step completes");
        drop(sink);

        let model_lane = tool_lane(&provider.requests());
        assert!(
            model_lane.contains(&format!("UNSET {var}")),
            "an undeclared credential must be absent from the child's environment — the \
             helper reports one line per variable, either SET or UNSET: {model_lane}"
        );
        assert!(
            !model_lane.contains(ORCHESTRATOR_SOURCE),
            "the reference must not ride the tool result: {model_lane}"
        );

        cleanup(&cell);
    }

    /// Condition "references-only" removed: a declared credential whose
    /// reference cannot be resolved fails the call before anything spawns —
    /// the model is told exactly that, and no record exists because no child
    /// ever ran. The generated-config half of the condition (references
    /// only, never values) is asserted in test 1; the escape battery's
    /// `an_unresolvable_reference_injects_nothing_and_spawns_nothing` owns
    /// the typed-error-at-spawn shape.
    #[tokio::test]
    async fn an_unresolvable_reference_spawns_no_child_and_names_the_failure_to_the_model() {
        let cell = cell(
            "unresolved",
            &[orchestrator_endpoint()],
            &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
        )
        .await;
        let var = endpoint_env_var(ORCHESTRATOR_ROLE);
        let dangling = Credential::new(
            "orchestrator-key",
            &var,
            SecretRef::Env {
                env: "SAYA_E2E_MISSING_SOURCE".to_owned(),
            },
        )
        .expect("the declared credential is well-shaped");
        let provider = ScriptProvider::new(vec![
            Turn::Tools(vec![run_program_call(&["digest", &var])]),
            Turn::Answer("done"),
        ]);
        let tool = cell.tool(vec![dangling], source_with(&[]));
        let plan = RunPlan::new(vec![runner_step("measure the environment")]).expect("well-shaped");
        let sink = sink(&cell);
        drive_step(&cell, &sink, tool, &provider, &plan, 0)
            .await
            .expect("the loop survives the refusal and finishes the step");
        drop(sink);

        let model_lane = tool_lane(&provider.requests());
        assert!(
            model_lane.contains("could not be resolved"),
            "the unresolved reference must be fed back to the model as the tool result: \
             {model_lane}"
        );
        assert!(
            model_lane.contains("orchestrator-key"),
            "the failure must name the credential: {model_lane}"
        );
        assert!(
            record_files(&cell).is_empty(),
            "nothing spawned — the runner never produced a record"
        );

        cleanup(&cell);
    }

    // -- 4: a role's credential is not visible to another role's child --------

    /// One run, two steps: step 0 runs as the reviewer role (only the
    /// reviewer credential declared), step 1 as the orchestrator role (only
    /// the orchestrator credential declared). Each child probes both env
    /// variables by name: its own is SET, the other role's is UNSET — and
    /// the generated config binds each role to its own run-scoped env
    /// variable.
    #[tokio::test]
    async fn a_role_a_credential_is_not_visible_to_a_child_running_as_role_b() {
        let bindings =
            EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary"), ("reviewer", "reviewer-ep")])
                .expect("shaped bindings");
        let cell = cell(
            "roles",
            &[orchestrator_endpoint(), reviewer_endpoint()],
            &bindings,
        )
        .await;
        let reviewer_var = endpoint_env_var("reviewer");
        let orchestrator_var = endpoint_env_var(ORCHESTRATOR_ROLE);
        let source = source_with(&[
            (ORCHESTRATOR_SOURCE, ORCHESTRATOR_VALUE),
            (REVIEWER_SOURCE, REVIEWER_VALUE),
        ]);
        let plan = RunPlan::new(vec![
            runner_step("act as the reviewer"),
            runner_step("act as the orchestrator"),
        ])
        .expect("well-shaped plan");
        let sink = sink(&cell);

        let reviewer_tool = cell.tool(vec![reviewer_credential()], source.clone());
        let reviewer_provider = ScriptProvider::new(vec![
            Turn::Tools(vec![run_program_call(&[
                "which",
                &reviewer_var,
                &orchestrator_var,
            ])]),
            Turn::Answer("done"),
        ]);
        drive_step(&cell, &sink, reviewer_tool, &reviewer_provider, &plan, 0)
            .await
            .expect("the reviewer step completes");

        let orchestrator_tool = cell.tool(vec![orchestrator_credential()], source.clone());
        let orchestrator_provider = ScriptProvider::new(vec![
            Turn::Tools(vec![run_program_call(&[
                "which",
                &reviewer_var,
                &orchestrator_var,
            ])]),
            Turn::Answer("done"),
        ]);
        drive_step(
            &cell,
            &sink,
            orchestrator_tool,
            &orchestrator_provider,
            &plan,
            1,
        )
        .await
        .expect("the orchestrator step completes");
        drop(sink);

        let reviewer_lane = tool_lane(&reviewer_provider.requests());
        assert!(
            reviewer_lane.contains(&format!("SET {reviewer_var}")),
            "the reviewer child must see its own role's variable: {reviewer_lane}"
        );
        assert!(
            reviewer_lane.contains(&format!("UNSET {orchestrator_var}")),
            "the reviewer child must not see the orchestrator's variable: {reviewer_lane}"
        );
        let orchestrator_lane = tool_lane(&orchestrator_provider.requests());
        assert!(
            orchestrator_lane.contains(&format!("SET {orchestrator_var}")),
            "the orchestrator child must see its own role's variable: {orchestrator_lane}"
        );
        assert!(
            orchestrator_lane.contains(&format!("UNSET {reviewer_var}")),
            "the orchestrator child must not see the reviewer's variable: {orchestrator_lane}"
        );

        // Neither raw value reached anything the run wrote down.
        sweep_run(&cell, ORCHESTRATOR_VALUE);
        sweep_run(&cell, REVIEWER_VALUE);

        // The generated config binds each role to its own env variable.
        let config = fs::read_to_string(cell.run.state().join("config/config.toml"))
            .expect("the generated config reads");
        assert!(
            config.contains(&format!("api_key = {{ env = \"{reviewer_var}\" }}")),
            "the reviewer role binds its own env reference: {config}"
        );
        assert!(
            config.contains(&format!("api_key = {{ env = \"{orchestrator_var}\" }}")),
            "the orchestrator role binds its own env reference: {config}"
        );

        cleanup(&cell);
    }

    // -- 5: the failure path scrubs too ----------------------------------------

    /// The child ran three times (one per bounded attempt), each time
    /// echoing the live credential; the provider failed after every attempt;
    /// the bound was spent and the run paused mid-flight. The journal and
    /// the store — the two records a resume reads — carry no credential
    /// material, in their bytes or their rows.
    #[tokio::test]
    async fn a_failed_run_s_journal_and_store_carry_no_credential_material() {
        let cell = cell(
            "failed",
            &[orchestrator_endpoint()],
            &EndpointBindings::new([(ORCHESTRATOR_ROLE, "primary")]).expect("shaped bindings"),
        )
        .await;
        let var = endpoint_env_var(ORCHESTRATOR_ROLE);
        let leak_call = run_program_call(&["leak", &var]);
        let provider = ScriptProvider::new(vec![
            Turn::Tools(vec![leak_call.clone()]),
            Turn::Fail,
            Turn::Tools(vec![leak_call.clone()]),
            Turn::Fail,
            Turn::Tools(vec![leak_call]),
            Turn::Fail,
        ]);
        let tool = cell.tool(
            vec![orchestrator_credential()],
            source_with(&[(ORCHESTRATOR_SOURCE, ORCHESTRATOR_VALUE)]),
        );
        let plan = RunPlan::new(vec![runner_step("keep failing")]).expect("well-shaped plan");
        let sink = sink(&cell);
        let error = drive_step(&cell, &sink, tool, &provider, &plan, 0)
            .await
            .expect_err("the step exhausts its bounded attempts");
        assert!(
            matches!(
                &error,
                EpisodeError::StepExhausted {
                    step: 0,
                    attempts: 3,
                    ..
                }
            ),
            "the spent bound must surface as the typed exhaustion: {error:?}"
        );
        assert_eq!(
            sink.state(),
            RunState::Paused,
            "a spent bound pauses the run"
        );
        drop(sink);

        // The journal tells the bounded-retry story, paused — with no
        // credential material anywhere in it.
        let events = Journal::open(cell.run.root())
            .read()
            .expect("the journal parses");
        assert_eq!(
            events,
            vec![
                RunEvent::RunStarted,
                RunEvent::PlanApproved { scopes: None },
                RunEvent::StepStarted { step: 0 },
                RunEvent::StepFailed { step: 0 },
                RunEvent::StepStarted { step: 0 },
                RunEvent::StepFailed { step: 0 },
                RunEvent::StepStarted { step: 0 },
                RunEvent::StepFailed { step: 0 },
                RunEvent::Paused {
                    reason: PauseReason::StepFailedAfterRetry,
                },
            ],
            "the journal must carry exactly the bounded-retry story"
        );

        // The store's rows exist — and carry no credential material.
        let run = RunStore::get_run(&*cell.store, &cell.run_id)
            .await
            .expect("the store reads")
            .expect("the run row exists");
        assert_eq!(run.status, RunStatus::Paused, "the run paused");
        assert!(
            !format!("{run:?}").contains(ORCHESTRATOR_VALUE),
            "the store's run row carries credential material: {run:?}"
        );
        let steps = RunStore::list_steps(&*cell.store, &cell.run_id)
            .await
            .expect("the store reads");
        assert_eq!(steps.len(), 1, "one step row");
        assert_eq!(steps[0].status, RunStepStatus::Failed, "the step failed");
        assert!(
            !format!("{:?}", steps[0]).contains(ORCHESTRATOR_VALUE),
            "the store's step row carries credential material: {:?}",
            steps[0]
        );

        // The byte sweep over the store, the journal, and the whole run
        // directory — the failure path is exactly where scrubbing is skipped.
        sweep_run(&cell, ORCHESTRATOR_VALUE);
        for secret in [
            "hunter2-marker-shape",
            "sk-live-shape-marker",
            "hunter2-url-marker",
        ] {
            sweep_run(&cell, secret);
        }

        // Non-vacuity: the credential was live in all three attempts, and
        // every attempt's captured output was scrubbed on its way to disk.
        let records = record_files(&cell);
        assert_eq!(records.len(), 3, "the three attempts each ran the child");
        for record in &records {
            let text = fs::read_to_string(record).expect("the record reads");
            assert!(
                !text.contains(ORCHESTRATOR_VALUE),
                "the failed run's record carries the credential: {record:?}"
            );
            assert!(
                text.contains("[redacted]"),
                "the failed attempt's record must show the redaction: {record:?}"
            );
        }

        cleanup(&cell);
    }
}
