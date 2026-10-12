use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
};

#[path = "provider/script.rs"]
mod script;
#[path = "provider/transport.rs"]
mod transport;

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
    pub fn held() -> Self {
        Self::script(false, 4)
    }
    pub fn checks() -> Self {
        Self::script(false, 1)
    }
    pub fn commands() -> Self {
        Self::script(false, 2)
    }
    pub fn conflicts() -> Self {
        Self::script(false, 3)
    }
    pub fn pair_conflicts() -> Self {
        Self::script(false, 5)
    }
    // Mode 6, for the tool/command parity tests: the parent issues
    // `cancel_child` and `message_child` for the unregistered agent `ghost`,
    // then reads `coding_status`, then answers. The final answer is held
    // until `release()`, so a test can read the owner's live job state while
    // the run is parked; every request body is captured in `bodies`.
    pub fn parity() -> Self {
        Self::script(false, 6)
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
