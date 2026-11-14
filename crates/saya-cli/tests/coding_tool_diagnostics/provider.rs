//! The provider fixture for the tool-diagnostics tests: the same TCP
//! transport contract as the coding_lifecycle provider (`Fixture::with_endpoint`
//! speaks OpenAI-compatible SSE to it), with the scripted modes owned here so
//! the lifecycle fixtures stay untouched.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

#[path = "provider_script.rs"]
pub(crate) mod script;
#[path = "provider_transport.rs"]
mod transport;

#[cfg(target_os = "macos")]
pub(crate) use script::TRANSCRIPT_STDOUT;
pub(crate) use script::{
    ARG_SENTINEL, OVERFLOW_CALLS, OVERFLOW_PATH, REASONING_SENTINEL, RESULT_SENTINEL,
    TRANSCRIPT_BIG_HEAD, TRANSCRIPT_BIG_TAIL, TRANSCRIPT_COMMAND, TRANSCRIPT_PATH,
    TRANSCRIPT_SECRET_ARGUMENT, TRANSCRIPT_SECRET_RESULT, overflow_body, unknown_tool_name,
};

pub struct Provider {
    pub endpoint: String,
    pub bodies: Arc<Mutex<Vec<String>>>,
    pub responses: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Provider {
    /// Delegate one child, join it, then refuse a second delegation.
    /// The child writes, reads the result, and attempts an unassigned write.
    pub fn diagnostics() -> Self {
        Self::script(100)
    }

    pub fn status() -> Self {
        Self::script(102)
    }

    /// A protocol-valid unknown name is rejected and the run recovers.
    pub fn unknown_tool() -> Self {
        Self::script(101)
    }

    /// One helper that writes, runs a real `shell` command, then writes a body
    /// larger than one recorded transcript field may keep.
    pub fn transcript() -> Self {
        Self::script(103)
    }

    /// One turn of many ranged reads whose results overflow the job-wide
    /// transcript budget, so later calls must keep compact facts only.
    pub fn overflow() -> Self {
        Self::script(104)
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
