//! The workspace the read-only tables are read against: a checkout whose own
//! `git` configuration names nothing that runs, so a `git` spelling is decided
//! by the catalogue. A rule that reads a file the workspace tools refuse — or
//! the repository's own configuration — needs a real root, so every row is
//! read against one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A scratch workspace, removed on drop. Its `.git` is a directory holding
/// `config`, because a repository's local configuration is what a vetted
/// `git` has to be read against.
pub(super) struct Workspace(PathBuf);

impl Workspace {
    /// A workspace whose repository configuration runs nothing.
    pub(super) fn new(tag: &str) -> Self {
        Self::with_config(tag, "[core]\n\trepositoryformatversion = 0\n")
    }

    /// A workspace whose repository configuration is exactly `config`.
    pub(super) fn with_config(tag: &str, config: &str) -> Self {
        let root = Self::root(tag);
        std::fs::create_dir_all(root.join(".git")).expect("the workspace must be creatable");
        std::fs::write(root.join(".git/config"), config).expect("the config must be writable");
        Self(root)
    }

    /// A workspace that holds no repository at all.
    pub(super) fn bare(tag: &str) -> Self {
        Self(Self::root(tag))
    }

    /// Adds a repository of its own at `dir`, below this workspace's root.
    pub(super) fn nested(&self, dir: &str, config: &str) -> &Self {
        let root = self.0.join(dir);
        std::fs::create_dir_all(root.join(".git")).expect("the nested repository must create");
        std::fs::write(root.join(".git/config"), config).expect("the config must be writable");
        self
    }

    fn root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "saya-read-only-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the workspace must be creatable");
        root
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
