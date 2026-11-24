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
//! The connector states that one bound in two words for the same second: the
//! pool's `acquire_timeout` and the `timeout` around the acquire in
//! `connect()` are both `query_timeout_seconds`, and a stalled handshake meets
//! both. Which of the two equal timers fires first is the runtime's to decide,
//! so under load either word is the bound's own: `PostgreSQL connection
//! failed` from the acquire, `PostgreSQL connection timed out` from the
//! timeout. The test asserts the bound was spent and that the session said the
//! profile it cannot open; it never asserts which of the two won a race.

use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
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

/// The connector's own words for that one bound, in the two places it states
/// it: the pool's acquire timing out, and the `connect()` timeout around the
/// acquire. Either may fire first; both mean the bound cut the stalled setup.
const BOUND_SENTENCES: [&str; 2] = [
    "PostgreSQL connection failed",
    "PostgreSQL connection timed out",
];

/// The silent server: it accepts each connection the client makes, holds it
/// open and answers nothing, so the PostgreSQL handshake that follows the TCP
/// connect is what stalls.
struct Silent {
    /// One token per connection accepted. Receiving one is this test's proof
    /// that the setup reached the server and stalled there rather than being
    /// refused at the socket.
    accepted: mpsc::Receiver<()>,
}

impl Silent {
    /// Bind the silent server and leave it accepting on its own thread. The
    /// accepted sockets live in that thread for as long as this handle's
    /// receiver does, so dropping the handle closes them.
    fn bind() -> (Self, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the silent server binds");
        let port = listener
            .local_addr()
            .expect("the silent server's address")
            .port();
        let (sender, accepted) = mpsc::channel();
        std::thread::spawn(move || {
            let mut held: Vec<TcpStream> = Vec::new();
            while let Ok((stream, _)) = listener.accept() {
                held.push(stream);
                if sender.send(()).is_err() {
                    return;
                }
            }
        });
        (Self { accepted }, port)
    }
}

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
fn stalling_session(label: &str) -> (PathBuf, LineSession, Silent) {
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
    let (silent, port) = Silent::bind();
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
        folder: Some(folder.clone()),
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
    let (root, parts, silent) = stalling_session("bound");
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

    // The setup reached the server and was answered nothing: the wait above was
    // a stalled handshake, not a refused port. Waited for, never slept for: the
    // accept lands as soon as the client's connect does.
    silent
        .accepted
        .recv_timeout(CEILING)
        .expect("the connector reached the silent server and stalled in its handshake");
    assert!(
        elapsed >= Duration::from_secs(QUERY_TIMEOUT_SECONDS),
        "the wait was the connector's own bound, not an instant refusal: {elapsed:?}"
    );
    let said = front
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let [notice] = said.as_slice() else {
        panic!("the operator reads exactly one notice about the profile: {said:?}");
    };
    let reason = notice
        .strip_prefix("cannot open profile 'demo': connection failed: ")
        .unwrap_or_else(|| panic!("the notice names the profile it cannot open: {notice:?}"));
    assert!(
        BOUND_SENTENCES.contains(&reason),
        "the operator reads the bound's own sentence: {notice:?}"
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
