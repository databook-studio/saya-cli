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

/// A symlink into the git directory names the same files. The check resolves
/// the path's existing prefix, so an alias spelling with no `.git` component
/// cannot hide a write the operator would otherwise be asked about.
#[cfg(unix)]
#[test]
fn a_symlink_into_the_git_directory_names_it() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new("symlink");
    fs::create_dir_all(sandbox.root().join(".git/hooks")).expect("git dir creates");
    fs::create_dir_all(sandbox.root().join("src")).expect("ordinary dir creates");
    symlink(
        sandbox.root().join(".git"),
        sandbox.root().join(".git-alias"),
    )
    .expect("the git alias creates");
    symlink(sandbox.root().join("src"), sandbox.root().join("src-alias"))
        .expect("the ordinary alias creates");

    assert!(
        targets_git_dir(Some(sandbox.root()), ".git-alias/hooks/pre-commit"),
        "the alias resolves into the git directory"
    );
    assert!(
        targets_git_dir(
            Some(sandbox.root()),
            &sandbox
                .root()
                .join(".git-alias/config")
                .display()
                .to_string()
        ),
        "an absolute alias spelling is the same file"
    );
    assert!(
        !targets_git_dir(Some(sandbox.root()), "src-alias/lib.rs"),
        "a symlink to ordinary content is not the git directory"
    );
}
