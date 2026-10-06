//! The launch posture derivation (`session_loop.rs`): every launch kind —
//! fresh, `--continue`, `--resume` — derives the status header's facts from
//! the universe the launch composed, and the deny journal stays a fresh-start
//! write. A resumed launch's universe composes the host lane wherever a root
//! binds (the same composer behind both paths), so a derivation guarded to
//! fresh starts leaves the header on `host:off` while the lane executes.

use super::session_loop::derive_launch_posture;
use crate::interactive::session_host::HostLaunch;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, CompactionMode, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ResolvedSessionDeny, ThemeChoice,
};
use saya_store::JournalEvent;
use std::path::Path;

const ID: &str = "20260101-000000-lnc0";
const DENIED: &str = "curl";

/// Removes the throwaway root when the fixture drops; declared after the
/// session runtime so the lock releases before the directory goes.
struct RootGuard(std::path::PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    state: SessionState,
    session: SessionRuntime,
    /// Kept last so the session's lock releases before the directory goes.
    _root: RootGuard,
}

impl Fixture {
    /// The recorded shape every launch kind loads: the two header facts are
    /// `#[serde(skip)]`, so any state arrives as the defaults (lane off, no
    /// deny list) no matter what the previous process ran under.
    fn state() -> SessionState {
        SessionState::new(ID, None, "test-model")
    }

    /// A fresh-shaped launch: `fresh: true` with the explicit statement, then
    /// the launch recomposition `run()` performs before deriving the posture.
    /// The launch carries the `--deny curl` refusal.
    fn build_fresh(tag: &str) -> Self {
        let root = root_for(tag);
        std::fs::create_dir_all(&root).expect("the fixture root");
        let sessions = root.join("sessions");
        let runtime = test_runtime(&[]);
        let project = project_dir(&root);
        let mut session = SessionRuntime::acquire(
            &runtime,
            Some(Path::new(&project)),
            true,
            None,
            ID,
            ApprovalPolicy::Ask,
            &sessions,
        )
        .expect("the fresh fixture session acquires");
        let launch = HostLaunch::from_deny_for_tests(vec![DENIED.to_owned()]);
        session
            .recompose_with_launch(&runtime, None, Some(&launch))
            .expect("the fresh launch recomposes");
        Self {
            state: Self::state(),
            session,
            _root: RootGuard(root),
        }
    }

    /// A resumed-shaped launch (`--resume`): `fresh: false`, the recorded pin
    /// rides the state, and the launch statement is gone — the universe is
    /// what `acquire` composed, deny list from the user-layer config alone.
    fn build_resumed(tag: &str, pin: Option<String>, deny: &[&str]) -> Self {
        let root = root_for(tag);
        std::fs::create_dir_all(&root).expect("the fixture root");
        let sessions = root.join("sessions");
        let runtime = test_runtime(deny);
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            pin.as_deref(),
            ID,
            ApprovalPolicy::Ask,
            &sessions,
        )
        .expect("the resumed fixture session acquires");
        let mut state = Self::state();
        state.workspace_root = pin;
        Self {
            state,
            session,
            _root: RootGuard(root),
        }
    }

    /// A `--continue`-shaped launch: `fresh: false` re-opening the most
    /// recent session — the same non-fresh composition path `--resume` takes.
    fn build_continued(tag: &str, pin: Option<String>, deny: &[&str]) -> Self {
        Self::build_resumed(tag, pin, deny)
    }
}

fn root_for(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "saya-launch-posture-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ))
}

fn project_dir(root: &std::path::Path) -> String {
    let project = root.join("project");
    std::fs::create_dir_all(&project).expect("the fixture project dir");
    project.display().to_string()
}

/// The runtime fixture: no profiles and no endpoints, so composition touches
/// nothing real; `deny` is the user-layer `[session_commands] deny` the
/// composer folds into every universe.
fn test_runtime(deny: &[&str]) -> crate::config::runtime::RuntimeConfig {
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
                compaction: CompactionMode::Auto,
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
            session_deny: ResolvedSessionDeny {
                programs: deny.iter().map(|name| (*name).to_owned()).collect(),
            },
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: std::path::PathBuf::from("/tmp/saya-launch-posture"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// The `DenyList` journal events, in order — the deny list the launch wrote.
fn deny_journal_events(session: &SessionRuntime) -> Vec<Vec<String>> {
    session
        .journal()
        .read()
        .expect("the journal reads")
        .into_iter()
        .filter_map(|event| match event {
            JournalEvent::DenyList { programs } => Some(programs),
            _ => None,
        })
        .collect()
}

/// A fresh launch derives the header facts from the recomposed universe —
/// lane composed where the workspace bound, deny list the launch stated —
/// and journals the deny list exactly once.
#[test]
fn a_fresh_launch_derives_the_header_facts_and_journals_the_deny_list() {
    let mut fx = Fixture::build_fresh("fresh");
    derive_launch_posture(&mut fx.state, &fx.session, true);
    assert_eq!(
        fx.state.host_composed,
        fx.session.universe().host_composed(),
        "the header's lane bit is the launch universe's fact"
    );
    assert!(
        fx.state.host_composed,
        "the explicit workspace bound a root: the lane composes"
    );
    assert_eq!(
        fx.state.denied_programs,
        fx.session.universe().deny_programs(),
        "the header's deny list is the launch universe's fact"
    );
    assert_eq!(fx.state.denied_programs, vec![DENIED.to_owned()]);
    assert_eq!(
        deny_journal_events(&fx.session),
        vec![vec![DENIED.to_owned()]],
        "a fresh start journals its deny list once"
    );
}

/// A resumed launch derives the header facts too: the acquired universe
/// composed the lane from the recorded pin, and its deny list (the user-layer
/// config's, folded in at composition) is what the header copies. The deny
/// journal writes nothing — the launch statement belonged to the previous
/// process, whose start journaled its own list.
#[test]
fn a_resumed_launch_derives_the_header_facts_from_its_composed_universe() {
    let root = root_for("resume-pin");
    let pin = project_dir(&root);
    let mut fx = Fixture::build_resumed("resume", Some(pin.clone()), &[DENIED]);
    derive_launch_posture(&mut fx.state, &fx.session, false);
    assert_eq!(
        fx.state.host_composed,
        fx.session.universe().host_composed(),
        "the header's lane bit is the resumed universe's fact"
    );
    assert!(
        fx.state.host_composed,
        "the recorded pin re-binds a root: the lane composes for the header"
    );
    assert_eq!(
        fx.state.denied_programs,
        fx.session.universe().deny_programs(),
        "the header's deny list is the resumed universe's fact"
    );
    assert_eq!(
        fx.state.denied_programs,
        vec![DENIED.to_owned()],
        "the config deny composed into the universe is what the header carries"
    );
    assert!(
        deny_journal_events(&fx.session).is_empty(),
        "a resumed launch journals no deny list"
    );
    drop(fx);
    let _ = std::fs::remove_dir_all(&root);
}

/// A continued launch derives the header facts from the unbound shape too:
/// the lane did not compose and the header must say so (a stale `true` never
/// survives), while the deny list still rides the laneless universe.
#[test]
fn a_continued_launch_derives_the_header_facts_and_journals_nothing() {
    let mut fx = Fixture::build_continued("continue", None, &[DENIED]);
    fx.state.host_composed = true;
    derive_launch_posture(&mut fx.state, &fx.session, false);
    assert_eq!(
        fx.state.host_composed,
        fx.session.universe().host_composed(),
        "the header's lane bit is the continued universe's fact"
    );
    assert!(
        !fx.state.host_composed,
        "no root binds on this continued launch: the header says the lane is off"
    );
    assert_eq!(
        fx.state.denied_programs,
        fx.session.universe().deny_programs(),
        "the deny list rides even where the lane did not compose"
    );
    assert_eq!(fx.state.denied_programs, vec![DENIED.to_owned()]);
    assert!(
        deny_journal_events(&fx.session).is_empty(),
        "a continued launch journals no deny list"
    );
}
