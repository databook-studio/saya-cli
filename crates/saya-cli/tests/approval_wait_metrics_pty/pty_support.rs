fn scratch_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-approval-wait-metrics-pty-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct OwnedChild(Option<Box<dyn portable_pty::Child + Send + Sync>>);

impl OwnedChild {
    fn new(child: Box<dyn portable_pty::Child + Send + Sync>) -> Self {
        Self(Some(child))
    }

    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        self.0
            .as_mut()
            .expect("owned child remains live")
            .try_wait()
    }

    fn disarm(&mut self) {
        self.0.take();
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct PtyReader(Option<thread::JoinHandle<()>>);

impl PtyReader {
    fn new(thread: thread::JoinHandle<()>) -> Self {
        Self(Some(thread))
    }

    fn join(mut self) {
        self.0
            .take()
            .expect("PTY reader remains live")
            .join()
            .expect("PTY reader joins after its owner closes");
    }
}

impl Drop for PtyReader {
    fn drop(&mut self) {
        if let Some(thread) = self.0.take() {
            let _ = thread.join();
        }
    }
}
