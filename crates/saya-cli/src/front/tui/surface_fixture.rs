//! The surface a scripted test drives: a real run over a throwaway folder,
//! against a scripted provider, opened by the launch's own opener
//! ([`open_surface`]) and driven through the port its commands are performed on.
//!
//! Nothing here fakes the session: the run is composed, opened and prepared by
//! the same code the launch runs, the turns are the engine's own, and the words
//! read back are the ones the provider actually received. What the fixture adds
//! is the throwaway tree, the scripted endpoint, and the port.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use saya_agent::ApprovalPolicy;
use saya_store::{FsSessionStore, SqliteStateStore};
use tokio::sync::mpsc;

use super::direct_run::DirectDeps;
use super::direct_trust::TrustStore;
use super::jobs_open::JobDeps;
use super::line_surface::SurfaceLine;
use super::live::LiveRuntime;
use super::msg::Msg;
use super::ops::OpsParts;
use super::ops_deps::OpsDeps;
use super::run_cmd::CmdPort;
use super::run_cmd_port::{PortParts, SessionPort};
use super::run_front::Forward;
use crate::GlobalOptions;
use crate::chat::ChatOps;
use crate::commands::coding::parent_turn_provider::Scripted;
use crate::interactive::SessionState;
use crate::interactive::session::line_chat::open_surface;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::turn_loop::surface_parts;
use crate::load_with_sources;
use crate::render::RenderFormat;
use crate::slash::user_commands::UserCommands;

/// The model the session opens on.
pub(crate) const MODEL: &str = "surface-model";

/// One fixture: its throwaway tree, the run the launch opened, and the port the
/// surface performs its commands through.
pub(crate) struct Fixture {
    pub(crate) port: Arc<SessionPort>,
    pub(crate) provider: Scripted,
    pub(crate) state: Arc<Mutex<SessionState>>,
    /// The line the surface drives: the run it is on now, its transcript and
    /// its rollover.
    pub(crate) line: Arc<SurfaceLine>,
    /// What the surface has said, in order: the notices a test reads back.
    words: Arc<Mutex<Vec<String>>>,
    /// Held for its drop, after everything else: the session's own directory
    /// lock, and the folder the run leased.
    _session: SessionRuntime,
    _root: RootGuard,
}

impl Fixture {
    /// The words the surface has said so far, drained: what a test asserts the
    /// transcript names.
    pub(crate) fn said(&self) -> Vec<String> {
        let mut said = self.words.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *said)
    }

    fn drain(&self, mut queue: mpsc::UnboundedReceiver<Msg>) {
        let words = Arc::clone(&self.words);
        tokio::spawn(async move {
            while let Some(msg) = queue.recv().await {
                let said = match msg {
                    Msg::Worker(worker) => match worker.direct {
                        Some(super::direct::DirectOutcome::Notice(words)) => words,
                        Some(super::direct::DirectOutcome::Error(words)) => words,
                        _ => worker.message,
                    },
                    _ => String::new(),
                };
                if !said.is_empty() {
                    words
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(said);
                }
            }
        });
    }

    /// The session's state, with a poisoned lock read as the value it last held.
    pub(crate) fn state(&self) -> MutexGuard<'_, SessionState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The request bodies the provider received, in the order they arrived.
    pub(crate) fn bodies(&self) -> Vec<String> {
        self.provider.bodies()
    }

    /// Waits until the provider has received `count` requests, and fails the
    /// test rather than hanging when it never does.
    pub(crate) async fn settle(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.provider.arrived() < count {
            assert!(
                Instant::now() < deadline,
                "the provider received {} of {count} requests",
                self.provider.arrived()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Performs one slash command on the surface, and waits for it to land: a
    /// command that moves the run is performed on the line's own thread, and the
    /// turn after it must run on the run the command prepared.
    pub(crate) async fn op(&self, command: &str) {
        self.port.op(super::ops_fixture::command(command));
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.line.busy() || self.pending() {
            assert!(
                Instant::now() < deadline,
                "the command the surface was given never landed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Whether an operation is still queued for the line's thread: the command
    /// just sent has been accepted but not yet started.
    fn pending(&self) -> bool {
        self.line.queued()
    }
}

/// A surface over a throwaway folder, answering from a scripted provider.
pub(crate) async fn surface(label: &str) -> Fixture {
    let provider = Scripted::start();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-surface-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("the fixture root creates");
    let root = root.canonicalize().expect("the fixture root resolves");
    let folder = root.join("repo");
    fs::create_dir(&folder).expect("the fixture folder creates");
    let programs = root.join("programs");
    fs::create_dir(&programs).expect("the fixture program directory creates");
    let config_home = root.join("config");
    fs::create_dir_all(&config_home).expect("the fixture config directory creates");
    let mut runtime = load_with_sources(
        &GlobalOptions::default(),
        &folder,
        &config_home,
        BTreeMap::new(),
    )
    .expect("the fixture resolves a runtime");
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.model = MODEL.into();
    runtime.resolved.ai.base_url = Some(provider.endpoint.clone());
    runtime.resolved.ai.api_key = None;
    runtime.resolved.ai.retry_delays_ms.clear();
    runtime.resolved.ai.timeout_seconds = 10;
    runtime.resolved.ai.idle_timeout_seconds = 10;
    runtime.resolved.ai.max_output_tokens = Some(4096);
    runtime.resolved.ai.context_window_tokens = Some(128_000);
    let effective = runtime
        .resolved
        .endpoints
        .get_mut(saya_config::ORCHESTRATOR_ROLE)
        .expect("the runtime resolves an orchestrator endpoint");
    // The endpoint an orchestrating turn reaches: the same scripted server, and
    // no model of its own — the session's own model is the one a request names,
    // which is what a `/model` change moves.
    effective.provider = saya_config::AiProvider::OpenaiCompatible;
    effective.base_url = Some(provider.endpoint.clone());
    effective.api_key = None;
    let runtime = Arc::new(runtime);
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
    // One session id per fixture: the job's own name, and the record's.
    let id = format!("surface-{label}");
    let session = SessionRuntime::acquire(
        &runtime,
        Some(&folder),
        true,
        None,
        &id,
        ApprovalPolicy::Ask,
        &root.join("session-dirs"),
    )
    .expect("the fixture's session acquires");
    // The session's own state, on the provider the scripted endpoint serves: a
    // state change re-resolves the provider and the endpoint through it, so the
    // two must agree with the runtime's own composition.
    let mut opening = SessionState::new(&id, None, MODEL);
    opening.provider = "openai_compatible".to_owned();
    opening.provider_endpoint = Some(provider.endpoint.clone());
    opening.provider_endpoint_bound = true;
    let state = Arc::new(Mutex::new(opening));
    let live = Arc::new(LiveRuntime::new(Arc::clone(&runtime)));
    let mut composed = surface_parts(
        &state.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        &session,
        &runtime,
        &state_db,
        RenderFormat::Text,
    );
    // The job's records go to this fixture's own root, never the operator's.
    composed.parts.state_root = Some(root.join("coding"));
    let (msgs, queue) = mpsc::unbounded_channel::<Msg>();
    let front = Arc::new(Forward::new(msgs));
    let opened = open_surface(
        composed.parts,
        &composed.record,
        Arc::clone(&front) as Arc<dyn crate::chat::ChatFront>,
    )
    .await
    .expect("the launch's run opens");
    let line = Arc::new(SurfaceLine::new(opened.into_line(), Arc::clone(&front)));
    let deps = Arc::new(OpsDeps::new(
        ChatOps::new(
            line.run_port().expect("the launch's run has a port"),
            line.context(true)
                .expect("the launch's run has a mode cell"),
        ),
        UserCommands::default(),
        Arc::clone(&live),
        Arc::clone(&state),
        OpsParts {
            store: FsSessionStore::new(root.join("session-store")),
            state_db: state_db.clone(),
            format: RenderFormat::Text,
            root: Some(folder.clone()),
            policy: session.policy(),
            journal: session.journal(),
            universe: session.universe(),
            recovery: session.recovery().clone(),
        },
    ));
    let jobs = Arc::new(JobDeps::new(
        Arc::clone(&live),
        state_db.clone(),
        Some(&folder),
        &id,
        session.policy().mode(),
    ));
    let direct = Arc::new(DirectDeps {
        runtime: Arc::clone(&live),
        state_db,
        format: RenderFormat::Text,
        trust: Arc::new(NoTrust) as Arc<dyn TrustStore>,
    });
    let port = Arc::new(SessionPort::new(PortParts {
        line: Arc::clone(&line),
        front: Arc::clone(&front),
        allow_bypass: true,
        direct,
        deps,
        options: GlobalOptions::default(),
        runtime: live,
        state: Arc::clone(&state),
        jobs,
    }));
    let fixture = Fixture {
        words: Arc::new(Mutex::new(Vec::new())),
        port,
        provider,
        state,
        line,
        _session: session,
        _root: RootGuard(root),
    };
    fixture.drain(queue);
    fixture
}

/// A trust store that binds nothing: these tests ask no trust question.
struct NoTrust;

impl TrustStore for NoTrust {
    fn bind(&self, _dir: &Path) -> Result<(), String> {
        Ok(())
    }
}

/// Removes the throwaway root when it drops, once the session is gone.
struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
