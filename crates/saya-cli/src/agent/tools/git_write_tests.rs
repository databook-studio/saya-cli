//! The shapes that name a repository's git directory: a `.git` component in
//! any position, and the directory a `.git` pointer file names.

use std::fs;
use std::path::{Path, PathBuf};

use super::targets_git_dir;

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("saya-gitrule-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("sandbox root creates");
        // A run's workspace root is canonical; a temp directory's spelling may
        // cross a symlink (`/var` on macOS).
        let root = fs::canonicalize(&root).expect("sandbox root resolves");
        Self { root }
    }

    fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_git_component_names_the_git_directory() {
    for path in [
        ".git",
        ".git/config",
        ".git/hooks/pre-commit",
        "src/.git/config",
        "src/../.git/config",
        ".GIT/config",
    ] {
        assert!(
            targets_git_dir(None, path),
            "{path} names the git directory"
        );
    }
}

#[test]
fn ordinary_repository_content_is_not_the_git_directory() {
    for path in [
        "src/lib.rs",
        "gitignore",
        ".github/workflows/ci.yml",
        "docs/git.md",
    ] {
        assert!(!targets_git_dir(None, path), "{path} is ordinary content");
    }
}

#[test]
fn a_git_pointer_files_gitdir_is_the_git_directory() {
    let sandbox = Sandbox::new("pointer");
    fs::create_dir_all(sandbox.root().join(".gitdirs/wt")).expect("gitdir creates");
    fs::create_dir_all(sandbox.root().join(".gitdirs/other")).expect("other creates");
    fs::write(sandbox.root().join(".git"), "gitdir: .gitdirs/wt\n").expect("pointer writes");

    assert!(
        targets_git_dir(Some(sandbox.root()), ".gitdirs/wt/config"),
        "the pointer's gitdir is the git directory"
    );
    assert!(
        targets_git_dir(
            Some(sandbox.root()),
            &sandbox
                .root()
                .join(".gitdirs/wt/config")
                .display()
                .to_string()
        ),
        "an absolute target inside the pointer's gitdir is the same file"
    );
    assert!(
        !targets_git_dir(Some(sandbox.root()), ".gitdirs/other/config"),
        "a sibling directory is not the pointer's gitdir"
    );
}

#[test]
fn a_git_directory_without_a_pointer_is_read_lexically() {
    let sandbox = Sandbox::new("directory");
    fs::create_dir_all(sandbox.root().join(".git/hooks")).expect("git dir creates");

    assert!(targets_git_dir(
        Some(sandbox.root()),
        ".git/hooks/pre-commit"
    ));
    assert!(!targets_git_dir(Some(sandbox.root()), "hooks/pre-commit"));
}

#[test]
fn a_checkout_with_no_git_names_only_a_git_component() {
    let sandbox = Sandbox::new("none");

    assert!(!targets_git_dir(Some(sandbox.root()), "src/lib.rs"));
    assert!(targets_git_dir(Some(sandbox.root()), ".git/config"));
}
