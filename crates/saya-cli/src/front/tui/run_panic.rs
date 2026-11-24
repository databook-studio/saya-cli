//! The hook that puts the terminal back when a panic walks out of the loop.
//!
//! A panic anywhere inside the runtime — in the update, in a command's port, in
//! a frame — must not leave the operator in raw mode on the alternate screen
//! with no cursor and no echo. The hook restores first and then chains the hook
//! it replaced, so the panic still reaches whatever was reporting panics before
//! the surface existed.

use std::panic::{self, PanicHookInfo};
use std::sync::{Arc, Mutex, PoisonError};

/// A panic hook of the standard library's own shape.
type Hook = Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static>;

/// The panic hook that restores the terminal before anything else looks at the
/// panic. Dropping it puts the previous hook back.
pub(crate) struct Restored {
    /// The hook that was installed before this one, until it is chained.
    previous: Arc<Mutex<Option<Hook>>>,
}

impl Restored {
    /// Installs the hook over `restorer`, when the screen has one. A screen that
    /// cannot be restored from anywhere leaves the process's hook alone.
    pub(crate) fn install(restorer: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        let previous: Arc<Mutex<Option<Hook>>> = Arc::new(Mutex::new(None));
        let Some(restore) = restorer else {
            return Self { previous };
        };
        let hook = panic::take_hook();
        *previous.lock().unwrap_or_else(PoisonError::into_inner) = Some(hook);
        let chain = Arc::clone(&previous);
        panic::set_hook(Box::new(move |info| {
            restore();
            let previous = chain.lock().ok().and_then(|mut slot| slot.take());
            if let Some(previous) = previous {
                previous(info);
            }
        }));
        Self { previous }
    }
}

impl Drop for Restored {
    fn drop(&mut self) {
        // A hook cannot be installed from a panicking thread, and during an
        // unwind the hook has already done its work — the chain is spent, it has
        // nothing to put back.
        if std::thread::panicking() {
            return;
        }
        let previous = self.previous.lock().ok().and_then(|mut slot| slot.take());
        if let Some(previous) = previous {
            let _ = panic::take_hook();
            panic::set_hook(previous);
        }
    }
}
