/// Kills the exact owned process group when its async owner is dropped early.
pub(super) struct GitProcessGroup {
    pid: u32,
    armed: bool,
}

impl GitProcessGroup {
    pub(super) fn new(pid: u32) -> Self {
        Self { pid, armed: true }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for GitProcessGroup {
    fn drop(&mut self) {
        if self.armed {
            let _ = crate::proc::kill_process_group(self.pid);
        }
    }
}
