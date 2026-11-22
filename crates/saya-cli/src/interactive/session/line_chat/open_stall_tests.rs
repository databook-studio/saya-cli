//! A connector whose setup stalls is cut by a bound, and the user is told.
//!
//! The old body's answering clock began before `prepare_turn` built the
//! connector, so a PostgreSQL server that accepted the socket and never
//! answered was cut by that clock (`agent/runtime_timer_tests/
//! answering_deadlines.rs::answering_deadline_starts_before_postgres_setup_...`).
//! On the live path the session resolves its connections *before* a run
//! exists, so the bound over connector setup is the connector's own: every
//! connector call — `connect()` included — runs under
//! `[limits] query_timeout_seconds`, and `session_run` states the failure the
//! way the surface has always stated it: one line naming the profile, then a
//! run that opens without the database instead of a session that cannot start.
//!
//! The twin: a stalled setup is cut by that bound, the operator reads the
//! bound's sentence, and the session still opens — never a hang.
//!
//! The connector states that sentence in two words for the same one second:
//! the pool's `acquire_timeout` and the query's own `timeout` are both
//! `query_timeout_seconds`, and the acquire is what a stalled handshake meets
//! first — `PostgreSQL connection failed`, wrapped by the session as the
//! profile it cannot open.

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{AgentEventSink, ApprovalPolicy, NoopEventSink};
use saya_store::SqliteStateStore;

use super::open::{LineSession, session_run};
use crate::GlobalOptions;
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, ChatFront, Question};
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::plugins::database::DatabaseBudget;
use crate::render::RenderFormat;

/// Load ceiling, not a product bound: how long the whole stalled setup may
/// take before this test calls it a hang. The product bound below is one
/// second, so the gap is wide enough to tell them apart.
const CEILING: Duration = Duration::from_secs(10);

/// The one bound the stalled connect is cut by, as this test states it.
const QUERY_TIMEOUT_SECONDS: u64 = 1;

/// A front that records what the session said and never asks anybody.
#[derive(Default)]
struct Recording {
    events: Mutex<Vec<String>>,
}

#[async_trait]
impl ChatFront for Recording {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(NoopEventSink)
    }

    async fn ask(&self, _question: &Question) -> Answer {
        Answer::Deny
    }

    fn emit(&self, event: ChatEvent) {
        if let ChatEvent::Notice { message } = event {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(message);
        }
    }
}

/// A folder and a configuration this test owns, whose one profile names a
/// PostgreSQL server that accepts the socket and then says nothing: the
/// connection completes and the protocol handshake stalls.
fn stalling_session(label: &str) -> (PathBuf, LineSession, TcpListener) {
    let root = std::env::temp_dir().join(format!(
        "saya-line-chat-stall-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    let folder = root.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let config_home = root.join("config").join("saya");
    std::fs::create_dir_all(&config_home).unwrap();
    std::fs::write(config_home.join("config.toml"), "# this row's own home\n").unwrap();
    let project = root.join("project.toml");
    std::fs::write(&project, "[ai]\nmodel = 'test-model'\n").unwrap();
    // The silent server: bound and left alone. A completed TCP handshake sits
    // in its backlog, so the client's `connect()` returns and the PostgreSQL
    // handshake that follows is what stalls.
    let silent = TcpListener::bind("127.0.0.1:0").expect("the silent server binds");
    let port = silent
        .local_addr()
        .expect("the silent server's address")
        .port();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.demo]\ntype = 'postgresql'\nhost = '127.0.0.1'\nport = {port}\n\
             database = 'synthetic'\nuser = 'saya'\nssl_mode = 'disable'\n"
        ),
    )
    .unwrap();
    let options = GlobalOptions {
        config: Some(project),
        connections: Some(connections),
        ..GlobalOptions::default()
    };
    let mut runtime: RuntimeConfig =
        load_with_sources(&options, &folder, &config_home, BTreeMap::new())
            .expect("the row's runtime resolves");
    // The bound this test pins: one second, so a stalled handshake is cut well
    // inside the ceiling above.
    runtime.resolved.query_timeout_seconds = QUERY_TIMEOUT_SECONDS;
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let parts = LineSession {
        session: String::from("stalled-profile-row"),
        folder: folder.clone(),
        runtime: Arc::new(runtime),
        store,
        mode: ApprovalPolicy::Ask,
        overrides: crate::agent::turn_config::PromptOverrides::default(),
        format: RenderFormat::Text,
        sink: None,
        budget: DatabaseBudget::defaults(),
        can_prompt: false,
        grants: saya_agent::SessionGrants::default(),
        state_root: Some(root.join("coding")),
        cancellation: None,
        tasks: None,
        facts: None,
        seed: Vec::new(),
    };
    (root, parts, silent)
}

#[tokio::test]
async fn a_stalled_connector_setup_is_cut_by_its_bound_and_the_operator_is_told() {
    let (root, parts, _silent) = stalling_session("bound");
    let front = Arc::new(Recording::default());
    let started = std::time::Instant::now();
    let run = tokio::time::timeout(
        CEILING,
        session_run(
            &parts,
            Arc::clone(&front) as Arc<dyn ChatFront>,
            String::from("stalled-profile-row"),
            &crate::chat::WorkspaceShape::Writable,
        ),
    )
    .await
    .expect("a stalled connector setup is cut by its bound, not a hang")
    .expect("the run opens without the profile it cannot reach");
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_secs(QUERY_TIMEOUT_SECONDS),
        "the wait was the connector's own bound, not an instant refusal: {elapsed:?}"
    );
    let said = front
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        said,
        vec![String::from(
            "cannot open profile 'demo': connection failed: PostgreSQL connection failed"
        )],
        "the operator reads the bound's own sentence, naming the profile"
    );
    assert!(
        run.connections.is_empty(),
        "no connection is attached: the run is a session with no database"
    );
    assert_eq!(
        run.skipped_primary.as_deref(),
        Some("demo"),
        "the surface is told which profile it is not talking to"
    );
    drop(root);
}
