//! The application a composition test opens a session's run over.
//!
//! A composition is given what a session resolved: the application over its
//! folder, the databases it connected, and the front it talks to. This fixture
//! builds the first two — an in-place root under `target/` and, when the test
//! asks for one, a seeded SQLite database named `primary` — and the tests state
//! the front, the mode and the session's name themselves.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, CancellationToken, NoopEventSink};
use saya_config::MemoryMode;
use saya_store::SqliteStateStore;

use super::compose::SessionRun;
use super::workspace_shape::WorkspaceShape;
use super::{ChatEvent, ChatFront, Question};
use crate::GlobalOptions;
use crate::agent::tools::consent::Answer;
use crate::commands::coding::CodingApplication;
use crate::connection::ConnectionRegistry;
use crate::load_with_sources;
use crate::plugins::database::DatabaseBudget;

#[path = "compose_fixture_database.rs"]
mod database;

#[allow(unused_imports)]
pub(crate) use database::{limit, sqlite_connection};

/// The in-place folder, the application over it, the databases the session
/// resolved — primary first — and the budget its turns spend.
pub(super) struct Fixture {
    pub(super) application: Arc<CodingApplication>,
    pub(super) connections: ConnectionRegistry,
    /// What a turn of this run spends once it uses the database: the table the
    /// fixture's resolved configuration states, so a test that narrows one
    /// dimension sees the live arming read its own number.
    budget: DatabaseBudget,
    root: RootGuard,
}

impl Fixture {
    /// The application and the run the session states over it, with the folder
    /// kept alive for as long as the caller holds it.
    pub(super) fn into_parts(
        self,
        front: Arc<dyn ChatFront>,
        session: &str,
        mode: saya_agent::ApprovalPolicy,
    ) -> Composed {
        let Fixture {
            application,
            connections,
            budget,
            root,
        } = self;
        Composed {
            application,
            run: SessionRun {
                front,
                session: session.to_owned(),
                mode,
                budget,
                shape: WorkspaceShape::Writable,
                connections,
                allow_query_data: true,
                memory: MemoryMode::Off,
                memory_reachable: false,
                capture: None,
                learning: None,
                tasks: None,
                facts: None,
                skipped_primary: None,
            },
            root,
        }
    }
}

/// What a test hands the composition, plus the folder that must outlive it.
pub(super) struct Composed {
    pub(super) application: Arc<CodingApplication>,
    pub(super) run: SessionRun,
    /// Held for its drop: the throwaway folder goes when the test does.
    pub(super) root: RootGuard,
}

/// An in-place session with no database configured.
pub(super) async fn in_place(case: &str, endpoint: &str) -> Fixture {
    build(case, endpoint, false, None, None, None, Ceilings::default()).await
}

/// The same, with one seeded SQLite database named `primary`: a chat that has a
/// database to read and to ask about.
pub(super) async fn in_place_with_database(case: &str, endpoint: &str) -> Fixture {
    build(case, endpoint, true, None, None, None, Ceilings::default()).await
}

/// The same, with a `[web_search]` backend configured: the session's run holds
/// the whole web lane, because the configuration — not a URL in the message —
/// is what advertises the web tools.
pub(super) async fn in_place_with_web_search(case: &str, endpoint: &str) -> Fixture {
    build(
        case,
        endpoint,
        false,
        None,
        Some("https://search.example"),
        None,
        Ceilings::default(),
    )
    .await
}

/// The same, with `memory` as the session's resolved memory mode: a chat whose
/// run can recall the claims its store holds and learn from its turns.
pub(super) async fn in_place_with_memory(
    case: &str,
    endpoint: &str,
    memory: MemoryMode,
) -> Fixture {
    build(
        case,
        endpoint,
        true,
        Some(memory),
        None,
        None,
        Ceilings::default(),
    )
    .await
}

/// The same, with the run's context byte budget stated by the test: the one
/// bound the untrusted-context projection reads, so a test can make the
/// projection outgrow it and see what the reserve keeps for history.
pub(super) async fn in_place_with_budget(
    case: &str,
    endpoint: &str,
    context_byte_budget: usize,
) -> Fixture {
    build(
        case,
        endpoint,
        false,
        None,
        None,
        Some(context_byte_budget),
        Ceilings::default(),
    )
    .await
}

/// The same, with the database budget its turns spend stated by the test: the
/// table the plugin arms on the turn's first database use.
pub(super) async fn in_place_with_database_ceilings(
    case: &str,
    endpoint: &str,
    ceilings: Ceilings,
) -> Fixture {
    build(case, endpoint, true, None, None, None, ceilings).await
}

/// The `[limits.database]` ceilings a test states for the session's run: every
/// field left `None` keeps the value the configuration resolved, so a test
/// narrows exactly the dimension it is about.
#[derive(Default)]
pub(super) struct Ceilings {
    pub(super) requests: Option<u64>,
    pub(super) tool_calls: Option<u64>,
    pub(super) elapsed_seconds: Option<u64>,
    pub(super) known_tokens: Option<u64>,
    /// The model's known request window: small enough and the turn refuses at
    /// preflight, before any provider request.
    pub(super) context_window_tokens: Option<u64>,
}

/// The same, with the session's memory on and the database budget its turns
/// spend stated by the test: the two facts the turn's own budget and its
/// post-turn learning pass read.
pub(super) async fn in_place_with_memory_and_ceilings(
    case: &str,
    endpoint: &str,
    memory: MemoryMode,
    ceilings: Ceilings,
) -> Fixture {
    build(case, endpoint, true, Some(memory), None, None, ceilings).await
}

/// The in-place folder's name inside the throwaway root.
const IN_PLACE_FOLDER: &str = "repo";

async fn build(
    case: &str,
    endpoint: &str,
    database: bool,
    memory: Option<MemoryMode>,
    web_search: Option<&str>,
    byte_budget: Option<usize>,
    ceilings: Ceilings,
) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-compose-{case}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("the fixture root creates");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .expect("the fixture root is private");
    let root = root.canonicalize().expect("the fixture root resolves");
    let folder = root.join(IN_PLACE_FOLDER);
    fs::create_dir(&folder).expect("the fixture folder creates");
    fs::write(folder.join("tracked.txt"), b"original\n").expect("the fixture file writes");
    let programs = root.join("programs");
    fs::create_dir(&programs).expect("the fixture program directory creates");
    let git = git_path();
    let mut runtime = load_with_sources(&GlobalOptions::default(), &folder, &root, BTreeMap::new())
        .expect("the fixture resolves a runtime");
    if let Some(mode) = memory {
        runtime.resolved.memory.mode = mode;
    }
    if let Some(endpoint) = web_search {
        runtime.resolved.web_search.backend = Some(saya_config::WebSearchBackend::Searxng {
            endpoint: endpoint.into(),
        });
    }
    if let Some(budget) = byte_budget {
        runtime.resolved.ai.context_byte_budget = budget;
    }
    if let Some(window) = ceilings.context_window_tokens {
        runtime.resolved.ai.context_window_tokens = Some(window);
    }
    if let Some(requests) = ceilings.requests {
        runtime.resolved.limits.database.requests = limit(requests);
    }
    if let Some(tool_calls) = ceilings.tool_calls {
        runtime.resolved.limits.database.tool_calls = limit(tool_calls);
    }
    if let Some(elapsed) = ceilings.elapsed_seconds {
        runtime.resolved.limits.database.elapsed_seconds = limit(elapsed);
    }
    if let Some(known_tokens) = ceilings.known_tokens {
        runtime.resolved.limits.database.known_tokens = limit(known_tokens);
    }
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.model = "composition".into();
    runtime.resolved.ai.base_url = Some(endpoint.into());
    runtime.resolved.ai.api_key = None;
    runtime.resolved.ai.retry_delays_ms.clear();
    runtime.resolved.ai.timeout_seconds = 5;
    runtime.resolved.ai.idle_timeout_seconds = 5;
    runtime.resolved.ai.max_output_tokens = Some(4096);
    let effective = runtime
        .resolved
        .endpoints
        .get_mut(saya_config::ORCHESTRATOR_ROLE)
        .expect("the runtime resolves an orchestrator endpoint");
    effective.provider = saya_config::AiProvider::OpenaiCompatible;
    effective.model = "composition".into();
    effective.base_url = Some(endpoint.into());
    effective.api_key = None;
    // The budget the run's turns spend, read off the configuration this fixture
    // just resolved — the same read production's session does, before the
    // runtime moves into the application.
    let budget = DatabaseBudget::of(&runtime);
    let application = CodingApplication::open_in_place(
        runtime,
        SqliteStateStore::new(root.join("state.sqlite3")),
        &folder,
        &root.join("state"),
        &git,
        &programs,
        &CancellationToken::new(),
    )
    .await
    .expect("the fixture application opens");
    let connections = if database {
        sqlite_connection(&root).await
    } else {
        ConnectionRegistry::new("")
    };
    // The budget the run's turns spend, read off the configuration this
    // fixture just resolved — the same read production's session does.
    Fixture {
        application,
        connections,
        budget,
        root: RootGuard(root),
    }
}

/// Removes the throwaway root when it drops.
pub(super) struct RootGuard(PathBuf);

impl RootGuard {
    /// The run's working checkout: the folder a job opened in place binds, and
    /// the root its shell composition proves the sandbox over.
    pub(super) fn checkout_root(&self) -> PathBuf {
        self.0.join(IN_PLACE_FOLDER)
    }
}

/// The `git` this machine runs, found on `PATH`: the fixture's Git service
/// holds no repository, so the binary only has to exist.
fn git_path() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH is set"))
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .expect("git is on this machine")
        .canonicalize()
        .expect("the git binary resolves")
}

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A front that answers every question itself, in the order it was told: the
/// first with `first`, every later one with `then`. It counts what it was put,
/// so a lane that asks after an answer is visible from the front's side.
pub(super) struct Answering {
    first: Answer,
    then: Answer,
    seen: AtomicUsize,
}

impl Answering {
    pub(super) fn new(first: Answer, then: Answer) -> Arc<Self> {
        Arc::new(Self {
            first,
            then,
            seen: AtomicUsize::new(0),
        })
    }

    /// The questions this front has been put so far.
    pub(super) fn seen(&self) -> usize {
        self.seen.load(Ordering::Acquire)
    }
}

#[async_trait]
impl ChatFront for Answering {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(NoopEventSink)
    }

    async fn ask(&self, _question: &Question) -> Answer {
        if self.seen.fetch_add(1, Ordering::AcqRel) == 0 {
            self.first
        } else {
            self.then
        }
    }

    fn emit(&self, _event: ChatEvent) {}
}
