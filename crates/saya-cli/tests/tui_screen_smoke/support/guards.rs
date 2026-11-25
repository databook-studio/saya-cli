// Scratch-home and child-lifecycle guards: a passed, failed or panicked case
// leaves no running `saya` and no files behind.

/// A throwaway HOME under the test's temp dir, tagged per test so parallel
/// cases in one binary cannot clobber each other.
fn scratch_home(tag: &str) -> PathBuf {
    let home =
        std::env::temp_dir().join(format!("saya-tui-screen-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// Kills the child on drop so a passed, failed or panicked test never leaks a
/// running `saya`.
struct ChildGuard {
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl ChildGuard {
    fn new(child: Box<dyn portable_pty::Child + Send + Sync>) -> Self {
        Self { child }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.try_wait();
        let _ = self.child.kill();
        let _ = self.child.try_wait();
    }
}

/// Removes the scratch HOME on drop so the run leaves no trace.
struct HomeGuard(PathBuf);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
