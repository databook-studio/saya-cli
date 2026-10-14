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
        let module = module_path!().replace("::", "-");
        let root = std::env::temp_dir().join(format!(
            "saya-extension-{module}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self {
            root: root.canonicalize().unwrap(),
        }
    }
    pub fn config(&self, script: &str) -> ExtensionConfig {
        // The framing scripts read a body with `read -r -n`, a bash builtin
        // (macOS /bin/sh is bash 3.2; Debian's is dash, which rejects it), so
        // bash is named explicitly wherever it exists.
        let shell = if std::path::Path::new("/bin/bash").exists() {
            "/bin/bash"
        } else {
            "/bin/sh"
        };
        let program = PathBuf::from(shell).canonicalize().unwrap();
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
