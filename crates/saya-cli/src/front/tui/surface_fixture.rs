//! The surface a scripted test drives: a real run over a throwaway folder,
//! against a scripted provider, opened by the launch's own opener and driven
//! through the port its commands are performed on.
//!
//! Nothing here fakes the session: the run is composed, opened and prepared by
//! the same code the launch runs, the turns are the engine's own, and the words
//! read back are the ones the provider actually received. What the fixture adds
//! is the throwaway tree, the scripted endpoint, and the port.
//!
//! The launch that builds one is [`super::surface_fixture_launch`], and what its
//! saved sessions offer is [`super::surface_fixture_resume`].

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::line_surface::SurfaceLine;
use super::msg::Msg;
use super::run_cmd::CmdPort;
use super::run_cmd_port::SessionPort;
use super::run_front::Forward;
use crate::commands::coding::parent_turn_provider::Scripted;
use crate::interactive::SessionState;
use crate::interactive::session::runtime::SessionRuntime;

#[allow(unused_imports)]
pub(crate) use super::surface_fixture_launch::{
    DATABASE, surface, surface_connected, surface_profile,
};

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
    /// The front the session talks to: a command's words are said through it.
    pub(super) front: Arc<Forward>,
    /// The folder this fixture's session worked in: the folder a record it saves
    /// pins.
    pub(super) folder: PathBuf,
    /// What the surface has said, in order: the notices a test reads back.
    pub(super) words: Arc<Mutex<Vec<String>>>,
    /// The rows the last `/sessions` (or bare `/resume`) opened the picker with,
    /// so a test reads what the surface offered rather than rebuilding it.
    pub(super) picked: Arc<Mutex<Option<Vec<super::view::SessionRow>>>>,
    /// Held for its drop, after everything else: the session's own directory
    /// lock, and the folder the run leased.
    pub(super) _session: SessionRuntime,
    pub(super) _root: RootGuard,
}

impl Fixture {
    /// The words the surface has said so far, drained: what a test asserts the
    /// transcript names. The read is synchronised first, so a sentence the queue
    /// still holds is never read as a sentence never said.
    pub(crate) async fn said(&self) -> Vec<String> {
        self.sync().await;
        let mut said = self.words.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *said)
    }

    /// Waits until the surface's own queue has been read up to this instant: a
    /// sentinel is handed to the same queue the drain reads, so everything said
    /// before it has landed when it comes back.
    async fn sync(&self) {
        const SENTINEL: &str = "\u{0}saya-fixture-sync";
        self.front.note(SENTINEL.to_owned());
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let landed = self
                .words
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .last()
                .is_some_and(|word| word == SENTINEL);
            if landed {
                self.words
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pop();
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the surface's own queue was never read"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    pub(super) fn drain(&self, mut queue: mpsc::UnboundedReceiver<Msg>) {
        let words = Arc::clone(&self.words);
        let picked = Arc::clone(&self.picked);
        tokio::spawn(async move {
            while let Some(msg) = queue.recv().await {
                let said = match msg {
                    Msg::Worker(worker) => match worker.direct {
                        Some(super::direct::DirectOutcome::Notice(words)) => words,
                        Some(super::direct::DirectOutcome::Error(words)) => words,
                        _ => worker.message,
                    },
                    // The picker's rows are not words: they are what the surface
                    // offered, kept where a test reads them.
                    Msg::Sessions(rows) => {
                        *picked.lock().unwrap_or_else(PoisonError::into_inner) = Some(rows);
                        String::new()
                    }
                    // A resume says one line, and it is the surface's own.
                    Msg::Resumed(view) => view.notice,
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
    pub(super) fn pending(&self) -> bool {
        self.line.queued()
    }

    /// The folder this fixture's session works in.
    pub(crate) fn folder(&self) -> PathBuf {
        self.folder.clone()
    }

    /// The port's own dependencies: the store the saved sessions live in, and the
    /// state the operation reads.
    pub(crate) fn deps(&self) -> Arc<super::ops_deps::OpsDeps> {
        Arc::clone(&self.port.deps)
    }
}

/// Removes the throwaway root when it drops, once the session is gone.
pub(super) struct RootGuard(pub(super) PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
