//! The application a composition test opens a session's run over.
//!
//! A composition is given what a session resolved: the application over its
//! folder, the databases it connected, and the front it talks to. This fixture
//! builds the first two — an in-place root under `target/` and, when the test
//! asks for one, a seeded SQLite database named `primary` — and the tests state
//! the front, the mode and the session's name themselves.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, CancellationToken, NoopEventSink};
use saya_config::MemoryMode;
use saya_connectors::{ConnectorOptions, SqliteConnector};
use saya_store::SqliteStateStore;
use saya_types::{DatabaseProfile, SqlDialect};

use super::compose::SessionRun;
use super::workspace_shape::WorkspaceShape;
use super::{ChatEvent, ChatFront, Question};
use crate::GlobalOptions;
use crate::agent::tools::consent::Answer;
use crate::commands::coding::CodingApplication;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use crate::load_with_sources;

/// The in-place folder, the application over it, and the databases the session
/// resolved — primary first.
pub(super) struct Fixture {
    pub(super) application: Arc<CodingApplication>,
    pub(super) connections: ConnectionRegistry,
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
            root,
        } = self;
        Composed {
            application,
            run: SessionRun {
                front,
                session: session.to_owned(),
                mode,
                shape: WorkspaceShape::Writable,
                connections,
                allow_query_data: true,
                memory: MemoryMode::Off,
                memory_reachable: false,
                capture: None,
                learning: None,
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
    build(case, endpoint, false, None).await
}

/// The same, with one seeded SQLite database named `primary`: a chat that has a
/// database to read and to ask about.
pub(super) async fn in_place_with_database(case: &str, endpoint: &str) -> Fixture {
    build(case, endpoint, true, None).await
}

/// The same, with `memory` as the session's resolved memory mode: a chat whose
/// run can recall the claims its store holds and learn from its turns.
pub(super) async fn in_place_with_memory(
    case: &str,
    endpoint: &str,
    memory: MemoryMode,
) -> Fixture {
    build(case, endpoint, true, Some(memory)).await
}

async fn build(case: &str, endpoint: &str, database: bool, memory: Option<MemoryMode>) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-compose-{case}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("the fixture root creates");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .expect("the fixture root is private");
    let root = root.canonicalize().expect("the fixture root resolves");
    let folder = root.join("repo");
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
    Fixture {
        application,
        connections,
        root: RootGuard(root),
    }
}

/// One seeded SQLite connection, named `primary` — the name a chat grants its
/// "yes for this run" under.
async fn sqlite_connection(root: &Path) -> ConnectionRegistry {
    let path = root.join("main.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .expect("the fixture database creates");
    sqlx::query("CREATE TABLE IF NOT EXISTS users (id INTEGER, name TEXT)")
        .execute(&seed)
        .await
        .expect("the fixture table creates");
    sqlx::query("INSERT INTO users VALUES (1, 'ada'), (2, 'grace')")
        .execute(&seed)
        .await
        .expect("the fixture rows insert");
    seed.close().await;
    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("the fixture database opens");
    // The entry carries the profile identity a stored claim is filed under, so
    // recall can resolve the profile it selects against — the same identity the
    // connection's own profile would hash to.
    let identity = crate::profile_identity::profile_identity(
        "primary",
        &DatabaseProfile::Sqlite {
            path: path.display().to_string(),
            read_only: true,
        },
        &root.join("connections.toml"),
    );
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(connector),
            dialect: SqlDialect::Sqlite,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
}

/// Removes the throwaway root when it drops.
pub(super) struct RootGuard(PathBuf);

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
