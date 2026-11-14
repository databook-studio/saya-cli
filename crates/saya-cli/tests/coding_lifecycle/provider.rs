use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
};

#[path = "provider/script.rs"]
mod script;
#[path = "provider/script_direct.rs"]
mod script_direct;
#[path = "provider/transport.rs"]
pub(crate) mod transport;

pub struct Provider {
    pub endpoint: String,
    pub calls: Arc<AtomicUsize>,
    pub overlap: Arc<AtomicBool>,
    pub saw_injection: Arc<AtomicBool>,
    pub bodies: Arc<Mutex<Vec<String>>>,
    status_gate: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Provider {
    pub fn new(injection: bool) -> Self {
        Self::script(injection, 0)
    }
    // Mode 111: both `helpers` calls in ONE turn, with each child's first turn
    // held until both are pending, so the two helpers' overlap is observable.
    pub fn overlapping_children() -> Self {
        Self::script(false, 111)
    }
    pub fn held() -> Self {
        Self::script(false, 4)
    }
    // Mode 1: the child runs a real failing command through `shell` (the
    // retired check tool's replacement), so the run keeps a real nonzero exit.
    pub fn checks() -> Self {
        Self::script(false, 1)
    }
    pub fn workspace_retry() -> Self {
        Self::script(false, 18)
    }
    pub fn post_edit() -> Self {
        Self::script(false, 30)
    }
    #[cfg(target_os = "macos")]
    pub fn post_edit_stale() -> Self {
        Self::script(false, 31)
    }
    #[cfg(target_os = "macos")]
    pub fn post_edit_without_runner() -> Self {
        Self::script(false, 32)
    }
    #[cfg(target_os = "macos")]
    pub fn post_edit_unavailable() -> Self {
        Self::script(false, 33)
    }
    #[cfg(target_os = "macos")]
    pub fn post_edit_observer_failure() -> Self {
        Self::script(false, 34)
    }
    pub fn post_edit_known_commit_failure() -> Self {
        Self::script(false, 35)
    }
    pub fn repair_controls() -> Self {
        Self::script(false, 21)
    }
    // Mode 60: one child whose first model stream is cut before its
    // terminator; the client retries it, then the parent follows the child up.
    pub fn child_stream_cut_once() -> Self {
        Self::script(false, 60)
    }
    // Mode 61: one child whose first two model streams are both cut, so a
    // single retry leaves its request exhausted; the parent awaits it and
    // answers.
    pub fn child_requests_exhausted() -> Self {
        Self::script(false, 61)
    }
    // Mode 109: one child whose model stream is cut after it had already
    // delivered a long reasoning flow — and cut again on every byte-identical
    // retry of that request. The cut carries content, so the identical request
    // is never re-sent: the child continues at the configured ceiling.
    pub fn child_stream_cut_after_reasoning() -> Self {
        Self::script(false, 109)
    }
    // Direct-mode preparation of the root working checkout W: the root writes
    // in W, prepares, and a resumed run replays the same generation (the
    // receipt is the test's own mutation target).
    pub fn direct_prepare_receipt() -> Self {
        Self::script(false, 89)
    }
    // Direct mode: a `helpers` child that completes without changing files; its
    // merge still moves the source generation.
    pub fn direct_prepare_empty_child() -> Self {
        Self::script(false, 87)
    }
    // Direct mode: a held child, so the root prepares while one is live.
    pub fn direct_prepare_active_child() -> Self {
        Self::script(false, 88)
    }
    // Direct mode: a failing check, a repair in W, and a replay of both.
    pub fn direct_prepare_repair() -> Self {
        Self::script(false, 90)
    }
    // Only the macOS sandbox-lane charge test scripts this sequence.
    #[cfg(target_os = "macos")]
    pub fn command_wait_sequence() -> Self {
        Self::script(false, 24)
    }
    // Mode 6, for the tool/command equivalence tests: the job has no approved write
    // path, so the parent's `helpers` and `read` calls are refused as unknown
    // and it answers. The final answer is held until `release()`, so a test can
    // read the owner's live job state while the run is parked; every request
    // body is captured in `bodies`.
    pub fn parity() -> Self {
        Self::script(false, 6)
    }
    // Mode 119: an in-place root edits one file twice, creates one, and
    // deletes one, so the job's before-image store records each first touch.
    pub fn before_images() -> Self {
        Self::script(false, 119)
    }
    // Mode 122: an in-place root attempts a write outside its stated scope.
    pub fn write_outside_scope() -> Self {
        Self::script(false, 122)
    }
    // Mode 123: an in-place root attempts a write outside the workspace.
    pub fn write_outside_workspace() -> Self {
        Self::script(false, 123)
    }
    // Mode 121: a start answers without calling a tool.
    pub fn answer_only() -> Self {
        Self::script(false, 121)
    }
    // Mode 120: a start reads the conflicted file and replaces the markers.
    pub fn resolve_conflict() -> Self {
        Self::script(false, 120)
    }
    pub fn coordinator_refusals() -> Self {
        Self::script(false, 37)
    }
    pub fn await_replay() -> Self {
        Self::script(false, 38)
    }
    pub fn await_empty() -> Self {
        Self::script(false, 39)
    }
    // Mode 7: the parent hands one piece at a time to three helpers through
    // synchronous `helpers` calls, then answers.
    pub fn one_path() -> Self {
        Self::script(false, 7)
    }
    // Mode 9: repeat a local read-only root call across the default
    // logical-request ceiling, then finish with a plain answer.
    pub fn repeated_status() -> Self {
        Self::script(false, 9)
    }
    // Mode 41: one read then an answer, each provider call reporting cached
    // input and reasoning tokens.
    pub fn usage_cached() -> Self {
        Self::script(false, 41)
    }
    // Mode 118: a read-only job asks for `tracked.txt`, then answers from the
    // result it received. The request bodies prove which tools were advertised.
    pub fn read_only() -> Self {
        Self::script(false, 118)
    }
    pub fn release(&self) {
        self.status_gate.store(true, Ordering::Release);
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
