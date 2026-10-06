//! `/rotate`'s runner against a real store and a real runtime: what a
//! completed rotation leaves on disk, what a refusal leaves untouched, and
//! what a failed swap keeps held.

use super::{RotateResult, rotate_session_with_id};
use crate::interactive::session_compact::{apply_with_continuation, plan as compact_plan};
use crate::interactive::session_resume::{SessionDefaults, block_on, resume_session};
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::{FsSessionStore, ParentLink, SessionStore};
use std::path::PathBuf;

const SUMMARY: &str = "older turns discussed orders";
const OLD: &str = "20260101-000000-old0";
const SUCCESSOR: &str = "20260101-000000-new0";

/// Removes the throwaway root when the fixture drops; declared after the
/// session runtime so the locks release before the directory goes.
struct RootGuard(std::path::PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    store: FsSessionStore,
    runtime: crate::config::runtime::RuntimeConfig,
    state: SessionState,
    session: SessionRuntime,
    /// Kept for the drop order: the guard is the last field, so the session
    /// locks release before the directory goes. The sessions root hangs off
    /// it, so a re-acquire re-opens the same tree.
    root: RootGuard,
}

/// The runtime fixture the interactive-level question tests build: a
/// resolved config with no profiles and no endpoints, so composition
/// touches nothing real.
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
        cache_scope: PathBuf::from("/tmp/saya-rotate-run"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

impl Fixture {
    /// A live session (`OLD`) over a nested store root: the tombstone dir
    /// lives at the root's parent, so every fixture owns its own.
    fn build(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-rotate-run-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("rotate fixture root");
        let sessions = root.join("sessions");
        let runtime = test_runtime();
        // `fresh: false` with no pin and no explicit statement composes an
        // unbound universe and never walks up to the test cwd's worktree.
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            None,
            OLD,
            ApprovalPolicy::Ask,
            &sessions,
        )
        .expect("the fixture session acquires");
        Self {
            store: FsSessionStore::new(&sessions),
            runtime,
            state: compacted_state(OLD),
            session,
            root: RootGuard(root),
        }
    }

    fn reacquire(&self, id: &str) -> Result<SessionRuntime, String> {
        SessionRuntime::acquire(
            &self.runtime,
            None,
            false,
            None,
            id,
            ApprovalPolicy::Ask,
            &self.root.0.join("sessions"),
        )
    }
}

/// A compacted parent through the production compaction path.
fn compacted_state(id: &str) -> SessionState {
    let mut state = SessionState::new(id, None, "m");
    for index in 0..6 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let history = state.provider_history();
    let plan = compact_plan(&state.turns, &history).expect("six turns plan");
    apply_with_continuation(&mut state, &plan, SUMMARY, &|_| None).expect("the summary applies");
    state
}

/// A completed rotation: the parent saved and unchanged, the successor
/// published with the store's lineage link, the state swapped to the tail
/// and carried summary — and nothing inherited: grants empty, recovery
/// off, the old lock released, the new lock held.
#[test]
fn a_completed_rotation_publishes_and_swaps() {
    let mut fx = Fixture::build("success");
    fx.session.policy().grants().grant("sql:demo");
    fx.session.recovery().set_enabled(true);
    let tail = fx.state.turns[fx.state.compacted_turns..].to_vec();
    let parent_record = fx.state.redacted();

    let outcome = rotate_session_with_id(
        &fx.store,
        &fx.runtime,
        &mut fx.state,
        &mut fx.session,
        SUCCESSOR,
    );
    let RotateResult::Rotated { old, new } = outcome else {
        panic!("a compacted session rotates: {outcome:?}");
    };
    assert_eq!(old, OLD);
    assert_eq!(new, SUCCESSOR);

    assert_eq!(fx.state.id, SUCCESSOR, "the successor is live");
    assert_eq!(fx.state.turns, tail, "the verbatim tail rides across");
    assert_eq!(fx.state.compacted_turns, 0);
    assert_eq!(
        fx.state.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the carried narrative is the live summary"
    );
    assert_eq!(
        fx.state.parent,
        Some(ParentLink {
            id: OLD.into(),
            rotation_id: SUCCESSOR.into()
        })
    );
    assert!(
        fx.session.policy().grants().is_empty(),
        "a successor inherits no grants"
    );
    assert!(
        !fx.session.recovery().is_enabled(),
        "a successor starts with recovery off"
    );

    let parent = block_on(fx.store.load(OLD))
        .expect("the parent loads")
        .expect("the parent record exists");
    assert_eq!(parent.id, parent_record.id);
    assert_eq!(parent.turns, parent_record.turns, "the parent is unchanged");
    assert_eq!(parent.parent, None, "the parent carries no link");

    let successor = block_on(fx.store.load(SUCCESSOR))
        .expect("the successor loads")
        .expect("the successor record exists");
    assert_eq!(
        successor.parent,
        Some(ParentLink {
            id: OLD.into(),
            rotation_id: SUCCESSOR.into()
        }),
        "the store wrote the lineage link"
    );

    assert!(
        fx.reacquire(OLD).is_ok(),
        "the old session's lock is released"
    );
    let error = fx
        .reacquire(SUCCESSOR)
        .err()
        .expect("the successor's lock is held");
    assert!(error.contains("already running"), "{error}");
}

/// An uncompacted session refuses with the sentence, and nothing rotated:
/// the session stands and no successor record exists.
#[test]
fn an_uncompacted_session_refuses_and_nothing_rotates() {
    let mut fx = Fixture::build("uncompacted");
    let mut plain = SessionState::new(OLD, None, "m");
    for index in 0..6 {
        plain.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    fx.state = plain;

    let outcome = rotate_session_with_id(
        &fx.store,
        &fx.runtime,
        &mut fx.state,
        &mut fx.session,
        SUCCESSOR,
    );
    let RotateResult::Refused(message) = outcome else {
        panic!("an uncompacted session refuses: {outcome:?}");
    };
    assert_eq!(
        message,
        "Nothing to carry over yet: run /compact first so the session has a summary."
    );
    assert_eq!(fx.state.id, OLD, "the session stands");
    assert!(
        block_on(fx.store.load(SUCCESSOR))
            .expect("the store loads")
            .is_none(),
        "no successor record was created"
    );
}

/// A record that would exceed the 4 MiB cap refuses with the
/// compact-further sentence: no successor exists, the session stands.
#[test]
fn an_over_cap_session_refuses_with_the_compact_further_sentence() {
    let mut fx = Fixture::build("over-cap");
    // The not-yet-summarised tail made huge: the continuation boundary
    // covers only the compacted prefix, so the record still validates.
    let huge = "x".repeat(5 << 20);
    fx.state.turns[4].user = huge.clone();

    let outcome = rotate_session_with_id(
        &fx.store,
        &fx.runtime,
        &mut fx.state,
        &mut fx.session,
        SUCCESSOR,
    );
    let RotateResult::Refused(message) = outcome else {
        panic!("an over-cap session refuses: {outcome:?}");
    };
    assert_eq!(
        message,
        "The successor would still exceed the 4 MiB session limit; compact further before rotating."
    );
    assert_eq!(fx.state.id, OLD, "the session stands");
    assert!(
        block_on(fx.store.load(SUCCESSOR))
            .expect("the store loads")
            .is_none(),
        "no successor record exists"
    );
    assert_eq!(fx.state.turns[4].user, huge, "the session is unchanged");
}

/// A live holder on the successor id stops the swap after the publish: the
/// successor is saved and resumable, the current session keeps its lock,
/// and the message names the resumable id.
#[test]
fn a_live_holder_on_the_successor_keeps_the_current_session() {
    let mut fx = Fixture::build("live-holder");
    let holder = fx
        .reacquire(SUCCESSOR)
        .expect("the holder acquires the successor id first");

    let outcome = rotate_session_with_id(
        &fx.store,
        &fx.runtime,
        &mut fx.state,
        &mut fx.session,
        SUCCESSOR,
    );
    let RotateResult::Incomplete(message) = outcome else {
        panic!("a held successor stops the swap: {outcome:?}");
    };
    assert!(
        message.contains("Rotation was not completed") && message.contains(SUCCESSOR),
        "the message names the resumable id: {message}"
    );
    let error = fx
        .reacquire(OLD)
        .err()
        .expect("the current session's lock is still held");
    assert!(error.contains("already running"), "{error}");

    let defaults = SessionDefaults {
        provider: "ollama".into(),
        model: "m".into(),
        allow_data_sharing: true,
        approval_mode: "ask".into(),
    };
    let loaded = resume_session(&fx.store, SUCCESSOR, &defaults)
        .expect("the successor resumes")
        .expect("the successor record exists");
    assert_eq!(
        loaded.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the carried summary is restored"
    );
    assert_eq!(loaded.compacted_turns, 0);
    drop(holder);
}
