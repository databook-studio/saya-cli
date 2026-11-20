//! The walk's test seam: one step between reading a directory entry and
//! reading that entry, so a test can make a listed path leave the tree the way
//! a concurrent cleanup does — and watch what the walk makes of it.
//!
//! The armed step lives on the calling thread, so tests running in parallel
//! never observe each other's steps. A product build carries no seam at all:
//! every call site is compiled only under `cfg(test)`.

use std::cell::RefCell;

/// Where the walk calls the armed step.
#[derive(Clone, Copy)]
pub(super) enum Step {
    /// A walk attempt is starting, before its root is opened.
    WalkStarted,
    /// A directory entry was just read, before the walk stats it.
    Listed,
}

/// The step a test arms.
type Armed = Box<dyn FnMut(Step, &str)>;

thread_local! {
    static STEP: RefCell<Option<Armed>> = const { RefCell::new(None) };
}

/// Arms `step`: it is called at every stage for the rest of this thread's test.
pub(super) fn arm(step: impl FnMut(Step, &str) + 'static) {
    STEP.with(|slot| *slot.borrow_mut() = Some(Box::new(step)));
}

/// Disarms the step, so a walk in the same test runs undisturbed.
pub(super) fn disarm() {
    STEP.with(|slot| *slot.borrow_mut() = None);
}

/// Calls the armed step, when one is armed.
pub(super) fn step(stage: Step, relative: &str) {
    STEP.with(|slot| {
        if let Some(armed) = slot.borrow_mut().as_mut() {
            armed(stage, relative);
        }
    });
}
