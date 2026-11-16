/// A private directory for one test's command files, removed when it drops.
struct CommandsDir(PathBuf);

impl CommandsDir {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-user-commands-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("the commands root");
        Self(root)
    }

    /// Writes one command file and returns its path.
    fn write(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, body).expect("the command file");
        path
    }
}

impl Drop for CommandsDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A directory that does not exist: the normal case of no `commands/` at all.
fn missing_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("saya-user-commands-{tag}-{}", std::process::id()))
}
