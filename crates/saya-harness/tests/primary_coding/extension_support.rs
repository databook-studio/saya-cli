use saya_harness::{
    coding::extensions::ExtensionConfig,
    runner::sandbox::{RunSandbox, SandboxProvision},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
pub struct Fixture {
    pub root: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-extension-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self {
            root: root.canonicalize().unwrap(),
        }
    }
    pub fn config(&self, script: &str) -> ExtensionConfig {
        let program = PathBuf::from("/bin/sh").canonicalize().unwrap();
        let sandbox = RunSandbox::new([self.root.clone()], []).unwrap();
        let provision = sandbox.prepare(program.parent().unwrap()).unwrap();
        let SandboxProvision::Proven { spawn, .. } = provision else {
            panic!(
                "extension lifecycle test requires proven host confinement; refusal is not a pass"
            )
        };
        ExtensionConfig::new(spawn, program, vec!["-c".into(), script.into()]).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
