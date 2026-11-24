//! When the root's shell composition runs.
//!
//! Composing the shell proves the sandbox, and the probe that proves it creates
//! and removes canary directories INSIDE the working checkout W. Any reader of
//! W that runs beside it can list an entry that is already gone — most
//! importantly the readiness walk behind the first model request, whose facts
//! would then differ from run to run. So the composition does not start at
//! build: it is held here, and started once the run no longer needs W to itself
//! (right after the first turn's context blocks are built) or on the first shell
//! call, whichever comes first. The tool is advertised from the first request
//! either way.

use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::OnceCell;

use crate::agent::tools::shell_policy::ShellTool;

/// The root's shell, as the run holds it.
pub(super) enum ShellState {
    /// The tool is composed and ready.
    Ready(ShellTool),
    /// The composition is held until the run starts it.
    Deferred(Deferred),
}

/// A composition that has not run yet: the one-shot closure, the blocking task
/// once it is started, and the composed tool once it resolved.
pub(super) struct Deferred {
    pending: Mutex<Pending>,
    /// The resolved composition. `Some(None)` is a composition that yielded no
    /// shell, which is as final as one that did.
    cell: OnceCell<Option<ShellTool>>,
}

#[derive(Default)]
struct Pending {
    compose: Option<Composition>,
    task: Option<tokio::task::JoinHandle<Option<ShellTool>>>,
}

/// The composition the root hands over at build.
type Composition = Box<dyn FnOnce() -> Option<ShellTool> + Send + 'static>;

impl Deferred {
    pub(super) fn new(compose: impl FnOnce() -> Option<ShellTool> + Send + 'static) -> Self {
        Self {
            pending: Mutex::new(Pending {
                compose: Some(Box::new(compose)),
                task: None,
            }),
            cell: OnceCell::new(),
        }
    }

    /// Starts the composition on a blocking thread, once. A composition that
    /// already started, or already resolved, is left alone, so every caller may
    /// ask at any time.
    pub(super) fn start(&self) {
        let mut pending = lock(&self.pending);
        if pending.task.is_some() {
            return;
        }
        let Some(compose) = pending.compose.take() else {
            return;
        };
        pending.task = Some(tokio::task::spawn_blocking(compose));
    }

    /// The composed tool, starting the composition when nothing has yet and
    /// waiting for it. A composition that yielded no shell, or panicked, is
    /// `None`.
    pub(super) async fn tool(&self) -> Option<&ShellTool> {
        self.start();
        self.cell
            .get_or_init(|| async {
                // Only the initializer reads the handle, and `start` stores one
                // whenever it consumed the composition, so this cannot miss a
                // composition another thread just started.
                let handle = lock(&self.pending).task.take();
                match handle {
                    Some(handle) => handle.await.ok().flatten(),
                    None => None,
                }
            })
            .await
            .as_ref()
    }

    /// The composed tool when it is already there, without starting the
    /// composition or waiting for it.
    pub(super) fn settled(&self) -> Option<&ShellTool> {
        self.cell.get().and_then(Option::as_ref)
    }

    /// Whether the composition has been started. The ordering the run
    /// guarantees — nothing runs before the first turn's context is built — is
    /// asserted through this in a test build; no product path asks.
    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(super) fn started(&self) -> bool {
        lock(&self.pending).task.is_some()
    }
}

fn lock<T>(slot: &Mutex<T>) -> MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}
