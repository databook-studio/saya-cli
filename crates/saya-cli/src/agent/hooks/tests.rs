//! The hook engine's battery: the `PreToolUse` decision (`tests/decision.rs`),
//! the two events that observe and the bounds around them
//! (`tests/observation.rs`), over the support here.
//!
//! Every test runs a real `/bin/sh` on the host lane over a scratch root, so
//! none of them needs a proven sandbox and none skips.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use saya_agent::{CancellationToken, ToolError, ToolExecutor};
use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, ResolvedHooks, resolve};
use saya_harness::runner::HostPolicy;
use serde_json::{Value, json};

use super::runner::{HookCaller, Hooks};

#[path = "tests/decision.rs"]
mod decision;
#[path = "tests/lifecycle.rs"]
mod lifecycle;
#[path = "tests/observation.rs"]
mod observation;
#[path = "tests/permission.rs"]
mod permission;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// A per-test scratch root: created, used, removed on drop.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "saya-hooks-{tag}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the hook test root must be creatable");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The hooks a fixture table resolves to, exactly as a surface would read
/// them off the resolved config.
fn hooks_of(toml: &str) -> ResolvedHooks {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(toml).expect("the hook fixture must parse"));
    resolve(input).expect("the hook fixture must resolve").hooks
}

/// The engine over a scratch root, for the tests that drive it directly.
fn engine(toml: &str, root: &TempRoot) -> Arc<Hooks> {
    Arc::new(
        Hooks::new(
            hooks_of(toml),
            root.path(),
            HostPolicy::default(),
            HookCaller::Job("job-1".into()),
            CancellationToken::new(),
        )
        .expect("a host-only engine constructs over a real root"),
    )
}

/// An inner executor that answers one call and records that it ran, so a test
/// can tell a blocked call from one that reached the surface.
struct Inner {
    ran: Arc<AtomicU64>,
    refuse: bool,
}

impl Inner {
    fn new() -> (Arc<Self>, Arc<AtomicU64>) {
        let ran = Arc::new(AtomicU64::new(0));
        (
            Arc::new(Self {
                ran: Arc::clone(&ran),
                refuse: false,
            }),
            ran,
        )
    }

    /// The same executor, but refusing everything the way a surface whose
    /// `[tools]` allow list removed a name does.
    fn refusing(ran: Arc<AtomicU64>) -> Arc<Self> {
        Arc::new(Self { ran, refuse: true })
    }
}

#[async_trait]
impl ToolExecutor for Inner {
    async fn execute(&self, _name: &str, _arguments: Value) -> Result<Value, ToolError> {
        self.ran.fetch_add(1, Ordering::Relaxed);
        if self.refuse {
            return Err(ToolError::UnsupportedTool);
        }
        Ok(json!({"ok": true}))
    }
}
