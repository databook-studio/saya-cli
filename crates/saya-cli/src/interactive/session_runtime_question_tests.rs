//! `settle_question` settles by the completing scope's identity: only the
//! scope `begin_question` handed out may settle itself, so a late completion
//! of a stale question can never restore or change the live question's
//! state. One table drives a real `SessionRuntime` and real `QuestionScope`
//! values through every transition.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::{FsSessionStore, SessionStore, SqliteStateStore};

use super::super::SessionState;
use super::super::session_resume::{SessionDefaults, block_on, resume_session};
use super::super::session_universe::SessionUniverse;
use super::super::tui::loop_tick::tick_resume;
use super::super::tui::types::App;
use super::{QuestionScope, SessionRuntime};

/// A named sessions root so parallel tests never share state directories.
fn temp_sessions_root_named(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-session-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            instructions: Default::default(),
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
        Self::build("question", "question-scope-walk")
    }

    fn build(tag: &str, id: &str) -> Self {
        let sessions_root = temp_sessions_root_named(tag);
        let runtime = test_runtime();
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            None,
            id,
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

/// `reacquire` is the one seam all three session-load paths share (the
/// headless `/resume`, the TUI slash `/resume`, and the TUI session picker
/// all call it), so its contract decides the outcome. One table, one walk: a
/// successful swap abandons the old question exactly like `abandon_question`
/// — the old token cancelled, the segment finished, Idle — and a refused
/// swap keeps the pending question: same scope, live token.
#[test]
fn reacquire_abandons_the_question_on_success_and_keeps_it_on_failure() {
    let table: Vec<Row> = vec![
        (
            "Pending + FAILED reacquire (the target's lock is held) -> still Pending: same scope, token live",
            Box::new(|h, label| {
                let a = h.begin(label);
                h.session.settle_question(&a, true);
                let blocker = SessionRuntime::acquire(
                    &h.runtime,
                    None,
                    false,
                    None,
                    "reacquire-held",
                    ApprovalPolicy::Ask,
                    &h.sessions_root,
                )
                .unwrap_or_else(|error| panic!("{label}: the blocker session acquires: {error}"));
                let error = h
                    .session
                    .reacquire(&h.runtime, None, "reacquire-held", ApprovalPolicy::Ask)
                    .expect_err("{label}: a held target refuses the swap");
                assert!(
                    error.contains("already running"),
                    "{label}: the refusal names the live holder: {error}"
                );
                let b = h.begin(label);
                assert!(
                    b.is_same_question(&a),
                    "{label}: the refused swap left the question pending"
                );
                assert!(
                    !b.cancellation().is_cancelled(),
                    "{label}: the refused swap left the token live"
                );
                h.session.settle_question(&b, false);
                drop(blocker);
            }),
        ),
        (
            "Pending + SUCCESSFUL reacquire -> the old token is cancelled; the next begin mints a DIFFERENT question",
            Box::new(|h, label| {
                let a = h.begin(label);
                h.session.settle_question(&a, true);
                h.session
                    .reacquire(&h.runtime, None, "reacquired", ApprovalPolicy::Ask)
                    .unwrap_or_else(|error| {
                        panic!("{label}: the swap to a saved session succeeds: {error}")
                    });
                assert!(
                    a.cancellation().is_cancelled(),
                    "{label}: the successful swap cancelled the old question's token"
                );
                let b = h.begin(label);
                assert!(
                    !b.is_same_question(&a),
                    "{label}: the abandoned question is gone; the next begin is a different question"
                );
                assert!(
                    !b.cancellation().is_cancelled(),
                    "{label}: the new question's token is live"
                );
                h.session.settle_question(&b, false);
            }),
        ),
        (
            "Active + SUCCESSFUL reacquire -> the old token is cancelled, Idle; the next begin is a fresh question",
            Box::new(|h, label| {
                let a = h.begin(label);
                h.session
                    .reacquire(&h.runtime, None, "reacquired-again", ApprovalPolicy::Ask)
                    .unwrap_or_else(|error| panic!("{label}: the second swap succeeds: {error}"));
                assert!(
                    a.cancellation().is_cancelled(),
                    "{label}: the successful swap cancelled the active question's token"
                );
                let b = h.begin(label);
                assert!(
                    !b.is_same_question(&a),
                    "{label}: the abandoned question is gone; the next begin is a different question"
                );
                assert!(
                    !b.cancellation().is_cancelled(),
                    "{label}: the new question's token is live"
                );
            }),
        ),
    ];
    let mut harness = Harness::build("reacquire", "reacquire-walk");
    for (label, step) in &table {
        step(&mut harness, label);
    }
    let root = harness.sessions_root.clone();
    drop(harness);
    let _ = std::fs::remove_dir_all(&root);
}

/// Saved-state cannot grant question authority. A session whose question is
/// pending clarification saves its state through the real save path; the
/// record round-trips the real loader (`resume_session`, the load all three
/// paths call), which carries no question — the lifecycle never rides
/// `SessionState` — and the runtime side of the successful load is the
/// shared `reacquire` seam. (Resuming the runtime's own id is refused by the
/// single-writer lock — the holder is this very process — so the swap, like
/// any real resume of another record, claims a fresh state directory.) The
/// loaded runtime's first `begin_question` therefore mints a fresh scope
/// from the trusted runtime config — a new token, a new identity, no
/// restored handle/time/count — and nothing auto-runs.
#[test]
fn a_saved_pending_state_grants_no_question_authority() {
    let root = temp_sessions_root_named("saved");
    let runtime = test_runtime();
    let sessions_root = root.join("sessions");
    let store = FsSessionStore::new(root.join("store"));
    let mut session = SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "saved-authority",
        ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("a real session runtime acquires");
    let old = session
        .begin_question(&runtime)
        .expect("the pending question begins");
    session.settle_question(&old, true);
    let state = SessionState::new("saved-authority", None, "test-model");
    block_on(store.save(state.redacted())).expect("the real save path persists the state");
    let loaded = resume_session(
        &store,
        "saved-authority",
        &SessionDefaults {
            provider: "ollama".into(),
            model: "test-model".into(),
            allow_data_sharing: true,
            approval_mode: "ask".into(),
        },
    )
    .expect("the record loads")
    .expect("the record exists");
    session
        .reacquire(
            &runtime,
            loaded.workspace_root.as_deref(),
            "resumed-fresh",
            ApprovalPolicy::Ask,
        )
        .expect("the successful load swaps in the loaded runtime");
    let fresh = session
        .begin_question(&runtime)
        .expect("the first begin on the loaded runtime mints a fresh question");
    assert!(
        !fresh.is_same_question(&old),
        "the saved narrative restored no question identity: the first begin is a fresh scope"
    );
    assert!(
        !fresh.cancellation().is_cancelled(),
        "the fresh scope carries a new, live token: no restored cancellation handle"
    );
    assert!(
        old.cancellation().is_cancelled(),
        "the old pending question was abandoned by the successful load, not carried across"
    );
    drop(session);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

/// The real picker resume surface: `tick_resume` is the loop tick the TUI
/// session picker arms (`overlays.pending_resume`, set exactly as
/// `application/picker.rs` does), and it drives the real loader plus the
/// shared `reacquire` seam. With a pending scope present beforehand, a
/// successful resume cancels the old token; a refused swap (the resumed
/// id's lock is held) leaves the pending question untouched.
#[test]
fn the_tui_picker_resume_abandons_the_question_on_success_and_keeps_it_on_failure() {
    // `SAYA_HISTORY=off` keeps `App::new` from reading the user's real
    // input history: the fixture touches no file outside its root.
    unsafe { std::env::set_var("SAYA_HISTORY", "off") };

    // A successful resume abandons the pending question.
    let root = temp_sessions_root_named("tui-resume");
    let runtime = test_runtime();
    let store = FsSessionStore::new(root.join("store"));
    let mut app = App::new(
        Vec::new(),
        Arc::new(runtime.clone()),
        SqliteStateStore::new(PathBuf::new()),
        Arc::new(SessionUniverse::empty()),
    );
    let mut session = SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "tui-resume-live",
        ApprovalPolicy::Ask,
        &root.join("sessions"),
    )
    .expect("the live session acquires");
    let mut state = SessionState::new("tui-resume-live", None, "test-model");
    let pending = session
        .begin_question(&runtime)
        .expect("the pending question begins");
    session.settle_question(&pending, true);
    block_on(store.save(SessionState::new("resumed-ok", None, "test-model").redacted()))
        .expect("the record saves");
    app.overlays.pending_resume = Some("resumed-ok".into());
    tick_resume(&mut app, &store, &mut state, &runtime, &mut session);
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("Resumed session resumed-ok")),
        "the picker resume said it resumed, not {:?}",
        app.transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        pending.cancellation().is_cancelled(),
        "the successful resume cancelled the old question's token"
    );
    drop(session);
    drop(app);
    drop(store);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(root);

    // A refused resume (the resumed id's lock is held) leaves it untouched.
    let root = temp_sessions_root_named("tui-resume-held");
    let runtime = test_runtime();
    let store = FsSessionStore::new(root.join("store"));
    let mut app = App::new(
        Vec::new(),
        Arc::new(runtime.clone()),
        SqliteStateStore::new(PathBuf::new()),
        Arc::new(SessionUniverse::empty()),
    );
    let mut session = SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "tui-resume-held-live",
        ApprovalPolicy::Ask,
        &root.join("sessions"),
    )
    .expect("the live session acquires");
    let blocker = SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "resumed-held",
        ApprovalPolicy::Ask,
        &root.join("sessions"),
    )
    .expect("the blocker session acquires");
    let mut state = SessionState::new("tui-resume-held-live", None, "test-model");
    let pending = session
        .begin_question(&runtime)
        .expect("the pending question begins");
    session.settle_question(&pending, true);
    block_on(store.save(SessionState::new("resumed-held", None, "test-model").redacted()))
        .expect("the record saves");
    app.overlays.pending_resume = Some("resumed-held".into());
    tick_resume(&mut app, &store, &mut state, &runtime, &mut session);
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("already running")),
        "the refused resume said why, not {:?}",
        app.transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        !pending.cancellation().is_cancelled(),
        "the refused resume left the pending question's token untouched"
    );
    let again = session
        .begin_question(&runtime)
        .expect("the question re-begins after the refused resume");
    assert!(
        again.is_same_question(&pending),
        "the refused resume left the question pending: the next begin re-begins the same question"
    );
    drop(blocker);
    drop(session);
    drop(app);
    drop(store);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(root);
}

/// A bounded wait for the background save `tick_resume` queues, so the
/// fixture root is removed only after the writer is done with it.
fn wait_for_background_save(store_root: &Path) {
    for _ in 0..200 {
        if std::fs::read_dir(store_root).is_ok_and(|entries| entries.count() > 0) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
