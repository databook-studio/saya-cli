//! Shared harness for the REPL budget-wording row: the child guard,

#[cfg(unix)]
pub(crate) struct InteractiveChildGuard(pub(crate) Option<std::process::Child>);

#[cfg(unix)]
impl Drop for InteractiveChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().unwrap().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

include!("harness/provider_io.rs");
include!("harness/mock_server.rs");
include!("harness/child.rs");
