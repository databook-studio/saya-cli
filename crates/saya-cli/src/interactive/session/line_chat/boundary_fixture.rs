//! The session a boundary test drives: a run open over a throwaway folder,
//! against a scripted provider, with the window the test states — and the front
//! its turns would print on (the outputs are the engine's own shape, stated by
//! the test).

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{AgentEventSink, ApprovalPolicy, CancellationToken, NoopEventSink};
use saya_config::MemoryMode;
use saya_store::SqliteStateStore;

use crate::GlobalOptions;
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, ChatFront, Question, RunComposition, SessionRun, WorkspaceShape};
use crate::commands::coding::CodingApplication;
use crate::commands::coding::parent_turn_provider::Scripted;
use crate::connection::ConnectionRegistry;
use crate::front::LineFront;
use crate::interactive::turn_summary::tally_handle;
use crate::load_with_sources;
use crate::render::RenderFormat;
/// The run a compaction rolls into: the session's next job, over the one
/// application.
pub(super) fn next_run(name: &str) -> SessionRun {
    SessionRun {
        front: Arc::new(Front),
        session: name.to_owned(),
        mode: ApprovalPolicy::Ask,
        budget: crate::plugins::database::DatabaseBudget::defaults(),
        shape: WorkspaceShape::Writable,
        connections: ConnectionRegistry::new(""),
        allow_query_data: true,
        memory: MemoryMode::Off,
        memory_reachable: false,
        capture: None,
        learning: None,
        tasks: None,
        facts: None,
        skipped_primary: None,
    }
}

/// A session's run over a throwaway folder, against a scripted provider, with
/// the window the test states.
pub(super) struct Fixture {
    pub(super) application: Arc<CodingApplication>,
    pub(super) provider: Scripted,
    /// The surface the run's turns print on: nothing here prints a turn — the
    /// outputs are the engine's own shape, stated by the test.
    pub(super) front: Arc<LineFront>,
    pub(super) run: RunComposition,
    /// Held for its drop, after the run: the folder goes when the lease is free.
    _root: RootGuard,
}

pub(super) async fn session(case: &str, window: u64) -> Fixture {
    let provider = Scripted::start();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-line-boundary-{case}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("the fixture root creates");
    let root = root.canonicalize().expect("the fixture root resolves");
    let folder = root.join("repo");
    fs::create_dir(&folder).expect("the fixture folder creates");
    let programs = root.join("programs");
    fs::create_dir(&programs).expect("the fixture program directory creates");
    let mut runtime = load_with_sources(&GlobalOptions::default(), &folder, &root, BTreeMap::new())
        .expect("the fixture resolves a runtime");
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.model = "line-boundary".into();
    runtime.resolved.ai.base_url = Some(provider.endpoint.clone());
    runtime.resolved.ai.api_key = None;
    runtime.resolved.ai.retry_delays_ms.clear();
    runtime.resolved.ai.timeout_seconds = 5;
    runtime.resolved.ai.idle_timeout_seconds = 5;
    runtime.resolved.ai.max_output_tokens = Some(4096);
    runtime.resolved.ai.context_window_tokens = Some(window);
    let effective = runtime
        .resolved
        .endpoints
        .get_mut(saya_config::ORCHESTRATOR_ROLE)
        .expect("the runtime resolves an orchestrator endpoint");
    effective.provider = saya_config::AiProvider::OpenaiCompatible;
    effective.model = "line-boundary".into();
    effective.base_url = Some(provider.endpoint.clone());
    effective.api_key = None;
    let application = CodingApplication::open_in_place(
        runtime,
        SqliteStateStore::new(root.join("state.sqlite3")),
        &folder,
        &root.join("state"),
        &git(),
        &programs,
        &CancellationToken::new(),
    )
    .await
    .expect("the fixture application opens");
    let run = RunComposition::compose(Arc::clone(&application), next_run(case))
        .await
        .expect("the session's run composes");
    run.open().await.expect("the session's run opens");
    Fixture {
        application,
        provider,
        front: Arc::new(LineFront::new(RenderFormat::Text, false, tally_handle())),
        run,
        _root: RootGuard(root),
    }
}

/// The `git` this machine runs, found on `PATH`: the fixture's Git service
/// holds no repository, so the binary only has to exist.
fn git() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH is set"))
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .expect("git is on this machine")
        .canonicalize()
        .expect("the git binary resolves")
}

/// A front that answers nothing: a scripted turn asks nobody.
struct Front;

#[async_trait]
impl ChatFront for Front {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(NoopEventSink)
    }

    async fn ask(&self, _question: &Question) -> Answer {
        Answer::Deny
    }

    fn emit(&self, _event: ChatEvent) {}
}

/// Removes the throwaway root when it drops.
struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
