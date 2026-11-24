//! What the surface's command table reads: the session's own operations, the
//! user's commands, the configuration in force, the session's state, and the
//! launch's parts — everything an operation needs and nothing it must reach for.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::live::LiveRuntime;
use crate::chat::ChatOps;
use crate::interactive::SessionState;
use crate::slash::user_commands::UserCommands;
use saya_store::{FsSessionStore, SqliteStateStore};

/// What an operation needs besides the chat layer's own table: the user's
/// commands, which answer for their names in `/help`, and everything the
/// operations read or move.
pub(crate) struct OpsDeps {
    /// The chat layer's own operations, over the run the session is on now: a
    /// rollover replaces that run, and the table is re-stated with it
    /// ([`OpsDeps::restate`]).
    chat: Mutex<ChatOps>,
    pub(crate) commands: UserCommands,
    /// The configuration this surface runs on, as `/reload` leaves it: the
    /// profile list `/connections` names is read here, never cached.
    pub(crate) runtime: Arc<LiveRuntime>,
    /// The session's state, as `/reload` leaves it: the session-command layer
    /// performs its reads and its statements on it, exactly as on the line front.
    pub(crate) state: Arc<Mutex<SessionState>>,
    /// The session's record store, and its store of state: `/status` reads the
    /// record's size, and the saved-session operations read both.
    pub(crate) store: FsSessionStore,
    pub(crate) state_db: SqliteStateStore,
    /// The folder the session bound, when it bound one: the workspace and
    /// instruction reads are about it.
    pub(crate) root: Option<std::path::PathBuf>,
    /// How a shared operation's printed output is rendered before it is filed.
    pub(crate) format: crate::render::RenderFormat,
    /// The session's approval policy — its mode and its one grant store, which
    /// is `Arc`-backed so `/allow` seeds the very store the next call reads.
    pub(crate) policy: saya_agent::SessionPolicy,
    /// The journal a seeded grant is written to, as the line front writes it.
    pub(crate) journal: Arc<saya_store::SessionJournal>,
    /// The session's universe: the deny list `/permissions` names, and the
    /// capabilities `/allow` composes against.
    pub(crate) universe: Arc<crate::interactive::session::universe::SessionUniverse>,
    /// The session's per-edit recovery store, which `/recovery` operates.
    pub(crate) recovery: crate::interactive::session::lifecycle::recovery::RecoveryHandle,
}

/// What a launch hands the table besides the session's own operations: the
/// session's stores, its policy and journal, its universe and its recovery
/// handle, and the folder it bound. Stated once here so the launch reads by
/// name and the table never reaches for a global.
pub(crate) struct OpsParts {
    pub(crate) store: FsSessionStore,
    pub(crate) state_db: SqliteStateStore,
    pub(crate) format: crate::render::RenderFormat,
    pub(crate) root: Option<std::path::PathBuf>,
    pub(crate) policy: saya_agent::SessionPolicy,
    pub(crate) journal: Arc<saya_store::SessionJournal>,
    pub(crate) universe: Arc<crate::interactive::session::universe::SessionUniverse>,
    pub(crate) recovery: crate::interactive::session::lifecycle::recovery::RecoveryHandle,
}

impl OpsDeps {
    /// The table's dependencies: the chat layer's own operations, the user's
    /// commands, the configuration in force, the session's state and the
    /// launch's own parts.
    pub(crate) fn new(
        chat: ChatOps,
        commands: UserCommands,
        runtime: Arc<LiveRuntime>,
        state: Arc<Mutex<SessionState>>,
        parts: OpsParts,
    ) -> Self {
        Self {
            chat: Mutex::new(chat),
            commands,
            runtime,
            state,
            store: parts.store,
            state_db: parts.state_db,
            format: parts.format,
            root: parts.root,
            policy: parts.policy,
            journal: parts.journal,
            universe: parts.universe,
            recovery: parts.recovery,
        }
    }

    /// The chat layer's table, as the run the session is on now states it.
    pub(crate) fn chat(&self) -> MutexGuard<'_, ChatOps> {
        self.chat.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Re-states the table over the run a rollover prepared: its port and the
    /// mode cell its commands move are the new run's, never the closed one's.
    pub(crate) fn restate(&self, chat: ChatOps) {
        *self.chat() = chat;
    }

    /// The session's state, with a poisoned lock read as the value it last held:
    /// a panicking reader must not take the session's own state with it.
    pub(crate) fn state(&self) -> MutexGuard<'_, SessionState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
