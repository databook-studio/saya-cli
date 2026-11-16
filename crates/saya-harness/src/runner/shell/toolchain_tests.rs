//! The toolchain resolver's bounds: resolved binaries reach the list, and
//! `$HOME`, the state root, and credential-shaped directories never do.

use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{MAX_TOOLCHAIN_DIRS, path_with_toolchain, toolchain_dirs};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "saya-toolchain-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the scratch root must be creatable");
        Self(path)
    }

    fn canonical(&self) -> PathBuf {
        self.0
            .canonicalize()
            .expect("the scratch root canonicalises")
    }

    fn executable(&self, relative: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("the bin dir");
        fs::write(&path, b"#!/bin/sh\nexit 0\n").expect("the fake binary");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("executable");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn host_path(dirs: &[&Path]) -> OsString {
    std::env::join_paths(dirs.iter().map(|dir| dir.as_os_str())).expect("a path list")
}

#[test]
fn resolved_toolchains_are_added_and_forbidden_trees_are_not() {
    let scratch = Scratch::new();
    let toolchain = scratch.executable("toolchain/bin/cargo");
    let toolchain_bin = toolchain.parent().expect("a bin dir").to_path_buf();
    scratch.executable(".ssh/bin/npm");
    scratch.executable("state/bin/rustc");
    scratch.executable(".config/bin/go");
    let home = scratch.canonical();
    let ssh_bin = home.join(".ssh/bin");
    let config_bin = home.join(".config/bin");
    let state_bin = home.join("state/bin");
    let path = host_path(&[&ssh_bin, &config_bin, &state_bin, &toolchain_bin]);
    let state_root = home.join("state");
    let dirs = toolchain_dirs(&path, &home, &[state_root]);

    assert!(
        dirs.contains(&toolchain_bin.canonicalize().expect("canonical")),
        "the resolved toolchain directory is added: {dirs:?}"
    );
    assert!(!dirs.contains(&home), "never $HOME itself");
    assert!(
        !dirs
            .iter()
            .any(|dir| dir.components().any(|part| part.as_os_str() == ".ssh")),
        "never a .ssh tree: {dirs:?}"
    );
    assert!(
        !dirs
            .iter()
            .any(|dir| dir.components().any(|part| part.as_os_str() == ".config")),
        "never a .config tree: {dirs:?}"
    );
    assert!(
        !dirs.iter().any(|dir| dir.starts_with(home.join("state"))),
        "never the state root: {dirs:?}"
    );
    assert!(
        dirs.len() <= MAX_TOOLCHAIN_DIRS,
        "the bound holds: {} <= {MAX_TOOLCHAIN_DIRS}",
        dirs.len()
    );
}

#[test]
fn the_sandbox_path_prepends_the_toolchain_directories() {
    let scratch = Scratch::new();
    let dirs = vec![scratch.canonical()];
    let path = path_with_toolchain(&dirs, "/usr/bin:/bin");
    let text = path.to_string_lossy();
    assert!(
        text.starts_with(&format!("{}:", scratch.canonical().display())),
        "the toolchain directory leads: {text}"
    );
    assert!(
        text.ends_with("/usr/bin:/bin"),
        "the base path follows: {text}"
    );
}

#[test]
fn a_rustup_toolchain_contributes_its_install_root() {
    let scratch = Scratch::new();
    let rustc = scratch.executable("toolchains/stable/bin/rustc");
    let toolchain_root = rustc
        .parent()
        .and_then(Path::parent)
        .expect("a toolchain root")
        .to_path_buf();
    let home = scratch.canonical();
    let path = host_path(&[rustc.parent().expect("a bin dir")]);
    let dirs = toolchain_dirs(&path, &home, &[]);
    assert!(
        dirs.contains(&toolchain_root.canonicalize().expect("canonical")),
        "the install root above bin is readable: {dirs:?}"
    );
}
