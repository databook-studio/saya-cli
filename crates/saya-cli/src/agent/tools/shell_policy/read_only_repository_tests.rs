//! The repository's own configuration, at the lane: a `git` spelling is vetted
//! only over a repository that runs nothing of its own, because the model's
//! own tools can write `.git/config` (see the `write` tool's battery).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::ApprovalPolicy;
use saya_config::CommandRules;
use serde_json::{Value, json};

use super::ShellPolicy;
use super::policy::Lane;
use crate::agent::tools::consent::{Caller, Operator};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A scratch checkout whose repository configuration is `config`, removed on
/// drop.
struct Checkout(PathBuf);

impl Checkout {
    fn new(tag: &str, config: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-ro-repo-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).expect("the checkout must be creatable");
        std::fs::write(root.join(".git/config"), config).expect("the config must be writable");
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Checkout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An operator that refuses every call and counts how often it was asked.
struct Counting(Mutex<usize>);

#[async_trait]
impl Operator for Counting {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        *self
            .0
            .lock()
            .expect("the counter is never held across an await") += 1;
        false
    }
}

/// A shell that decides for `root` under `ask`, with an operator present.
fn reviewing(root: &Path) -> ShellPolicy {
    let operator = Arc::new(Counting(Mutex::new(0)));
    let prompt = super::prompt(operator, Caller::Session);
    ShellPolicy::new(
        ApprovalPolicy::Ask,
        Some(prompt),
        true,
        CommandRules::default(),
        Some(root.to_path_buf()),
    )
}

/// A repository whose configuration names a program keeps the question: the
/// call reaches the operator, and their answer stands.
#[tokio::test]
async fn a_repository_that_runs_programs_keeps_the_question() {
    for config in [
        "[core]\n\tfsmonitor = ./hook.sh\n",
        "[diff \"driver\"]\n\ttextconv = ./hook.sh\n",
        "[filter \"driver\"]\n\tprocess = ./hook.sh\n",
    ] {
        let checkout = Checkout::new("running", config);
        let shell = reviewing(checkout.path());
        let call = json!({ "command": "git status" });
        assert_eq!(
            shell.lane(&call),
            Lane::Host(false),
            "a repository that runs a program is not a vetted read: {config:?}"
        );
        assert!(shell.would_ask(&call), "it asks: {config:?}");
        assert!(!shell.approve(&call).await, "the operator's answer stands");
    }
}

/// A repository that runs nothing is vetted as before, so the refusal is about
/// what the configuration does and never about `git` itself.
#[tokio::test]
async fn a_repository_that_runs_nothing_skips_the_question() {
    let checkout = Checkout::new("benign", "[core]\n\trepositoryformatversion = 0\n");
    let shell = reviewing(checkout.path());
    let call = json!({ "command": "git status" });
    assert_eq!(shell.lane(&call), Lane::Host(true));
    assert!(!shell.would_ask(&call));
    assert!(shell.approve(&call).await);
}
