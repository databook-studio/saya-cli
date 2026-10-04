//! `settle_question` settles by the completing scope's identity: only the
//! scope `begin_question` handed out may settle itself, so a late completion
//! of a stale question can never restore or change the live question's
//! state. One table drives a real `SessionRuntime` and real `QuestionScope`
//! values through every transition.

use std::path::PathBuf;

use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};

use super::{QuestionScope, SessionRuntime};

fn temp_sessions_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-session-question-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            profile_name: None,
            profile: None,
            investigation_budgets: Default::default(),
            ai: ResolvedAi {
                provider: AiProvider::Ollama,
                model: "test-model".into(),
                base_url: None,
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
                timeout_seconds: 60,
                idle_timeout_seconds: 90,
                max_output_tokens: 4096,
                max_output_tokens_is_default: true,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                compaction: saya_config::CompactionMode::Auto,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: Default::default(),
                turns: Some(4),
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-session-question"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

struct Harness {
    runtime: crate::config::runtime::RuntimeConfig,
    session: SessionRuntime,
    sessions_root: PathBuf,
    /// Every scope the walk has begun, in order: the stale rows settle one
    /// of these by index rather than minting a synthetic one.
    begun: Vec<QuestionScope>,
}

impl Harness {
    fn new() -> Self {
        let sessions_root = temp_sessions_root();
        let runtime = test_runtime();
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            None,
            "question-scope-walk",
            ApprovalPolicy::Ask,
            &sessions_root,
        )
        .expect("a real session runtime acquires");
        Self {
            runtime,
            session,
            sessions_root,
            begun: Vec::new(),
        }
    }

    fn begin(&mut self, label: &str) -> QuestionScope {
        let scope = self
            .session
            .begin_question(&self.runtime)
            .unwrap_or_else(|error| panic!("{label}: the question begins: {error}"));
        self.begun.push(scope.clone());
        scope
    }

    fn begin_refused(&mut self, label: &str) {
        let error = match self.session.begin_question(&self.runtime) {
            Ok(_) => panic!("{label}: the overlapping begin refuses while a question is active"),
            Err(error) => error,
        };
        assert!(
            error.contains("already active"),
            "{label}: the refusal names the live question: {error}"
        );
    }
}

/// One table row: the label said on failure, and the step it drives.
type Row = (&'static str, Box<dyn Fn(&mut Harness, &str)>);

/// One table, one walk: every transition `settle_question` may take, keyed
/// by WHICH scope completed — the scope `begin_question` handed out settles
/// itself, a stale scope's completion is ignored outright. The rows share
/// one real session and evolve its state in order; each row's asserts carry
/// the row label.
#[test]
fn settle_question_transitions_by_completing_scope_identity() {
    let table: Vec<Row> = vec![
        (
            "Idle + begin -> Active, the scope's token live",
            Box::new(|h, label| {
                let a = h.begin(label);
                assert!(
                    !a.cancellation().is_cancelled(),
                    "{label}: a begun question's token is live"
                );
            }),
        ),
        (
            "Active + overlapping begin -> refused, state unchanged",
            Box::new(|h, label| {
                h.begin_refused(label);
                let a = h.begun[0].clone();
                assert!(
                    !a.cancellation().is_cancelled(),
                    "{label}: the refused begin left the active question untouched"
                );
            }),
        ),
        (
            "Active + settle(same, clarification=true, token live) -> Pending; the next begin re-begins the SAME question",
            Box::new(|h, label| {
                let a = h.begun[0].clone();
                h.session.settle_question(&a, true);
                let b = h.begin(label);
                assert!(
                    b.is_same_question(&a),
                    "{label}: pending clarification re-begins the same question"
                );
                assert!(
                    !b.cancellation().is_cancelled() && !a.cancellation().is_cancelled(),
                    "{label}: the re-begun question keeps its live token"
                );
            }),
        ),
        (
            "Active + settle(same, clarification=false) -> Idle; the next begin mints a DIFFERENT question",
            Box::new(|h, label| {
                let a = h.begun[0].clone();
                h.session.settle_question(&a, false);
                let c = h.begin(label);
                assert!(
                    !c.is_same_question(&a),
                    "{label}: the settled-out question is gone; the next begin is a different question"
                );
            }),
        ),
        (
            "Active + settle(same, clarification=true, token cancelled) -> Idle",
            Box::new(|h, label| {
                let c = h.begun[2].clone();
                c.cancellation().cancel();
                h.session.settle_question(&c, true);
                let d = h.begin(label);
                assert!(
                    !d.is_same_question(&c),
                    "{label}: a cancelled settle cannot pend; the next begin is a different question"
                );
                assert!(
                    !d.cancellation().is_cancelled(),
                    "{label}: the new question's token is live"
                );
            }),
        ),
        (
            "Active(current) + settle(STALE other scope, clarification=true) -> ignored: current stays Active, its token stays live",
            Box::new(|h, label| {
                let stale = h.begun[2].clone();
                let current = h.begun[3].clone();
                h.session.settle_question(&stale, true);
                h.begin_refused(label);
                assert!(
                    !current.cancellation().is_cancelled(),
                    "{label}: the stale completion did not cancel the live question"
                );
                // The live question is still `current`: a same-scope
                // clarification=false settle lands (so the next begin mints
                // a fresh question), which only happens when the live scope
                // was this one.
                h.session.settle_question(&current, false);
                h.begin(label);
            }),
        ),
        (
            "Pending + abandon -> Idle and the old token is cancelled",
            Box::new(|h, label| {
                let e = h.begun[4].clone();
                h.session.settle_question(&e, true);
                h.session.abandon_question();
                assert!(
                    e.cancellation().is_cancelled(),
                    "{label}: abandoning the pending question cancels its token"
                );
                let f = h.begin(label);
                assert!(
                    !f.is_same_question(&e),
                    "{label}: abandon left Idle; the next begin is a different question"
                );
            }),
        ),
        (
            "Pending + settle(stale) -> still Pending, the pending scope and token untouched",
            Box::new(|h, label| {
                let f = h.begun[5].clone();
                h.session.settle_question(&f, true);
                let stale = h.begun[4].clone();
                h.session.settle_question(&stale, true);
                let g = h.begin(label);
                assert!(
                    g.is_same_question(&f),
                    "{label}: the stale settle was ignored; the question is still pending"
                );
                assert!(
                    !g.cancellation().is_cancelled(),
                    "{label}: the stale settle neither cancelled nor finished the pending question"
                );
                assert!(
                    stale.cancellation().is_cancelled(),
                    "{label}: the stale scope stays settled-out; nothing resurrected it"
                );
            }),
        ),
    ];
    let mut harness = Harness::new();
    for (label, step) in &table {
        step(&mut harness, label);
    }
    let root = harness.sessions_root.clone();
    drop(harness);
    let _ = std::fs::remove_dir_all(&root);
}
