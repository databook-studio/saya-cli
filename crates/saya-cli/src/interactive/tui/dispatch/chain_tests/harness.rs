const STARTED_UNIX_MS: i64 = 1_700_000_000_000;

/// Body text written into the fixture's AGENTS.md; `/context` must name the
/// file and never this text.
const CONTEXT_SENTINEL: &str = "SENTINEL-INSTRUCTION-BODY";

/// Removes the throwaway root when the fixture drops. Declared after the
/// session runtime in the fixture so the lock file is released before the
/// directory goes.
struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One fixture: the exact pieces the real `dispatch` call takes (the shape
/// `loop_tick::pending` passes), backed by a throwaway root.
struct ChainFixture {
    app: App,
    state: SessionState,
    session: SessionRuntime,
    store: FsSessionStore,
    runtime: RuntimeConfig,
    /// The launch options the loop's `SetupInputs` carries — `/reload` reads
    /// these for its same-launch-options re-read. Default everywhere except
    /// the reload tests, which point `--config` at a fixture file.
    launch_options: crate::cli::GlobalOptions,
    root: RootGuard,
}

impl ChainFixture {
    fn build(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-chain-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("chain test root");
        let mut runtime = unused_runtime();
        // The composed investigations root: every investigation operation the
        // dispatch runs writes under this fixture's own root, never the
        // machine's real data directory.
        runtime.investigations_root = root.join("investigations");
        runtime.connections.profiles.insert(
            "demo".to_string(),
            saya_types::DatabaseProfile::Sqlite {
                path: root.join("demo.sqlite3").display().to_string(),
                read_only: true,
            },
        );
        let session = SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            "chain-test",
            ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the chain test session acquires");
        Self {
            app: idle_app(),
            state: SessionState::new("chain-test", None, "model"),
            session,
            store: FsSessionStore::new(root.join("session-store")),
            runtime,
            launch_options: crate::cli::GlobalOptions::default(),
            root: RootGuard(root),
        }
    }

    /// The unbound variant: no explicit root, no pin, and `fresh: false` so
    /// no walk-up runs (a walk-up would bind this repo's own worktree from
    /// the test cwd). The pre-workspace resume shape — nothing binds, which
    /// is what the no-root refusals must be tested against.
    fn build_unbound(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-chain-unbound-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("unbound chain test root");
        let runtime = unused_runtime();
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            None,
            "chain-test",
            ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the unbound chain test session acquires");
        Self {
            app: idle_app(),
            state: SessionState::new("chain-test", None, "model"),
            session,
            store: FsSessionStore::new(root.join("session-store")),
            runtime,
            launch_options: crate::cli::GlobalOptions::default(),
            root: RootGuard(root),
        }
    }
}

/// The real `dispatch`, called with the fixture's pieces exactly as the
/// live loop does.
fn dispatch_line(line: &str, fx: &mut ChainFixture) -> Dispatch {
    super::dispatch(
        line,
        &mut fx.app.transcript,
        &fx.app.profiles,
        &mut fx.state,
        &fx.runtime,
        &fx.store,
        &fx.app.state_db,
        RenderFormat::Text,
        &mut fx.app.last_query,
        &fx.app.captured,
        fx.app.agent_captures.gap,
        &mut fx.session,
    )
}

/// One plausible line per registered command name; file destinations live
/// under the fixture's root. A new registry entry without a line here fails
/// the test naming the entry to fill in.
fn representative_line(name: &str, root: &std::path::Path) -> String {
    let path = |file: &str| root.join(file).display().to_string();
    let line = match name {
        "connect" => "/connect demo",
        "coding" => "/coding export chain-sentinel",
        "connections" => "/connections",
        "include" => "/include demo",
        "exclude" => "/exclude demo",
        "provider" => "/provider",
        "model" => "/model",
        "privacy" => "/privacy",
        "approvals" => "/approvals",
        "mode" => "/mode",
        "schema" => "/schema",
        "sql" => "/sql SELECT 1",
        "export" => return format!("/export --snapshot {}", path("export.csv")),
        "report" => return format!("/report {}", path("report.md")),
        "investigation" => "/investigation save chain-sentinel --sql SELECT 1 --connection demo",
        "investigations" => "/investigations",
        "chart" => return format!("/chart line {}", path("chart.png")),
        "explain" => "/explain SELECT 1",
        "clear" => "/clear",
        "compact" => "/compact",
        "history" => "/history",
        "sessions" => "/sessions",
        "notes" => "/notes",
        "rotate" => "/rotate",
        "preferences" => "/preferences",
        "recovery" => "/recovery",
        "resume" => "/resume 20260101-000000-abc",
        "setup" => "/setup",
        "reload" => "/reload",
        "columns" => "/columns",
        "contracts" => "/contracts",
        "contract" => "/contract",
        "remember" => "/remember users description orders-table",
        "forget" => "/forget 0000",
        "queue" => "/queue",
        "confirm" => "/confirm 0000",
        "reject" => "/reject 0000",
        "approve-all" => "/approve-all",
        "run" => "/run survey the data",
        "runs" => "/runs",
        "allow" => "/allow none",
        "context" => "/context",
        "grants" => "/grants",
        "doctor" => "/doctor",
        "usage" => "/usage",
        "workspace" => "/workspace",
        "thinking" => "/thinking",
        "tasks" => "/tasks",
        "help" => "/help",
        "exit" => "/exit",
        "quit" => "/quit",
        other => panic!("no representative line for /{other}: add one to chain_tests"),
    };
    line.to_string()
}
