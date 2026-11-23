//! Test support for the surface's command table: one fixture configuration, the
//! session's own state, and the chat layer's table over a port that records.
//!
//! The point of the recording port is the negative the chat layer already
//! proves: no command in this table enters the run. What these tests read is the
//! words the surface answers with, and the session facts an operation moved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, ApprovalPolicy};

use super::live::LiveRuntime;
use super::ops::{OpsDeps, OpsParts};
use crate::agent::tools::shell_policy::ModeCell;
use crate::chat::OpsContext;
use crate::chat::{ChatOps, ChatOutcome};
use crate::chat::{PortError, Prepared, RunPort};
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::interactive::SessionState;
use crate::interactive::session::runtime::SessionRuntime;
use crate::slash::user_commands::UserCommands;
use crate::slash::{SlashCommand, parse_slash_command};
use saya_store::{FsSessionStore, SqliteStateStore};

/// The profile every fixture names.
pub(crate) const PROFILE: &str = "local";

/// One fixture: its own tree, the surface's own state and what its table needs.
pub(crate) struct Fixture {
    pub(crate) deps: OpsDeps,
    pub(crate) state: Arc<Mutex<SessionState>>,
    pub(crate) port: Arc<RecordingRun>,
}

impl Fixture {
    /// A fresh tree with a SQLite profile in it, and the surface's table over it.
    pub(crate) fn new(label: &str) -> Self {
        let root = workspace_root()
            .join("target/tmp-ops")
            .join(format!("{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the fixture's tree is made");
        let database = root.join("data.sqlite3");
        std::fs::write(&database, b"").expect("the fixture's database is made");
        let connections = root.join("connections.toml");
        std::fs::write(
            &connections,
            format!(
                "[profiles.{PROFILE}]\ntype = 'sqlite'\npath = '{}'\n",
                database.display()
            ),
        )
        .expect("the fixture's connections file is written");
        let options = crate::cli::GlobalOptions {
            connections: Some(connections),
            ..Default::default()
        };
        let user = root.join("user-config");
        std::fs::create_dir_all(&user).expect("the fixture's user config dir is made");
        let runtime: RuntimeConfig = load_with_sources(&options, &root, &user, BTreeMap::new())
            .expect("the fixture runtime loads");
        let runtime = Arc::new(LiveRuntime::new(Arc::new(runtime)));
        let state = Arc::new(Mutex::new(SessionState::new("fx-1", None, "sonnet")));
        let port = Arc::new(RecordingRun::default());
        let chat = ChatOps::new(
            Arc::clone(&port) as Arc<dyn RunPort>,
            OpsContext::new(ModeCell::new(ApprovalPolicy::Ask), false),
        );
        // The session itself, acquired in the fixture's own tree: the parts the
        // table reads are its own, exactly as the launch takes them.
        let session = SessionRuntime::acquire(
            &runtime.current(),
            Some(&root),
            true,
            None,
            "fx-1",
            ApprovalPolicy::Ask,
            &root.join("session-dirs"),
        )
        .expect("the fixture's session acquires");
        let parts = OpsParts {
            store: FsSessionStore::new(root.join("session-store")),
            state_db: SqliteStateStore::new(root.join("state")),
            format: crate::render::RenderFormat::Text,
            root: Some(root.clone()),
            policy: session.policy(),
            journal: session.journal(),
            universe: session.universe(),
            recovery: session.recovery().clone(),
        };
        let deps = OpsDeps::new(
            chat,
            UserCommands::default(),
            runtime,
            Arc::clone(&state),
            parts,
        );
        Self { deps, state, port }
    }

    /// The session's state, with a poisoned lock read as the last value it held.
    pub(crate) fn state(&self) -> MutexGuard<'_, SessionState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A run port that records every call and answers none of them with a turn.
#[derive(Default)]
pub(crate) struct RecordingRun {
    seen: Mutex<Vec<&'static str>>,
}

impl RecordingRun {
    /// Every call that reached the port, in order.
    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RunPort for RecordingRun {
    async fn prepare(&self) -> Result<Prepared, String> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("prepare");
        Ok(Prepared::default())
    }

    async fn next_turn(
        &self,
        _sink: Arc<dyn AgentEventSink>,
        _prompt: String,
    ) -> Result<ChatOutcome, PortError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("next_turn");
        Ok(ChatOutcome::Answered)
    }

    async fn cancel_turn(&self) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("cancel_turn");
    }

    async fn close(&self) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("close");
    }
}

/// One command, parsed the way a surface parses it.
pub(crate) fn command(text: &str) -> SlashCommand {
    parse_slash_command(text)
        .unwrap_or_else(|error| panic!("`{text}` should parse: {error}"))
        .unwrap_or_else(|| panic!("`{text}` should be a slash command"))
}

/// The workspace root this crate is built in: `crates/saya-cli`'s two parents.
pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate sits two levels under the workspace root")
        .to_path_buf()
}
