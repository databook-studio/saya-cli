//! The host's toolchain directories, for a sandboxed child.
//!
//! A sandboxed `cargo test` needs the compiler, its standard library, and the
//! programs a build invokes — none of which live in the checkout. This module
//! resolves the directories of the toolchains the host's own `PATH` names and
//! hands them to the sandbox as **read-only, executable** roots; the checkout
//! stays the only writable tree.
//!
//! The bounds are explicit:
//! - at most [`MAX_TOOLCHAIN_DIRS`] directories, in resolution order;
//! - only programs that actually resolve on the host's `PATH`;
//! - never `$HOME` itself, the Saya state root, or any directory under
//!   `.ssh`, `.aws`, `.gnupg`, `.config`, or `.docker`.
//!
//! The directory list is computed once per process from the real environment;
//! the same list feeds the sandbox profile and the child's `PATH`, so the two
//! cannot disagree about where a toolchain lives.

use std::{
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// At most this many toolchain directories reach the sandbox.
pub const MAX_TOOLCHAIN_DIRS: usize = 16;

/// The programs whose directories a sandboxed build may need.
const PROGRAMS: &[&str] = &[
    "cargo", "rustc", "rustup", "node", "npm", "npx", "python3", "pip3", "go", "uv", "make",
];

/// Directory names that are never added, wherever they appear in a path.
const FORBIDDEN_COMPONENTS: &[&str] = &[".ssh", ".aws", ".gnupg", ".config", ".docker"];

/// The toolchain directories for this process: the host's `PATH`, `HOME`, and
/// state roots read from the environment.
pub fn toolchain_dirs_for_process() -> Vec<PathBuf> {
    let host_path = env::var_os("PATH").unwrap_or_default();
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let mut state_roots = vec![home.join(".saya")];
    if let Some(database) = env::var_os("SAYA_STATE_DB")
        .map(PathBuf::from)
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        state_roots.push(database);
    }
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(target_os = "macos")]
            {
                home.join("Library/Application Support")
            }
            #[cfg(not(target_os = "macos"))]
            {
                home.join(".local/share")
            }
        });
    state_roots.push(data_home.join("saya"));
    toolchain_dirs(&host_path, &home, &state_roots)
}

/// The pure resolver: the directories of `host_path`'s resolved toolchains,
/// canonicalised, filtered, deduplicated, and bounded.
pub fn toolchain_dirs(host_path: &OsStr, home: &Path, state_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for program in PROGRAMS {
        let Some(binary) = resolve_on_path(host_path, program) else {
            continue;
        };
        if let Some(parent) = binary.parent() {
            push_dir(&mut dirs, parent.to_path_buf(), home, state_roots);
        }
        if let Some(root) = rust_toolchain_root(&binary) {
            push_dir(&mut dirs, root, home, state_roots);
        }
    }
    dirs
}

/// The child's `PATH`: the toolchain directories first, then the base shell
/// path. An unjoinable path falls back to the base alone rather than failing
/// a spawn.
pub fn path_with_toolchain(dirs: &[PathBuf], base: &str) -> std::ffi::OsString {
    let mut paths: Vec<PathBuf> = dirs.to_vec();
    paths.extend(env::split_paths(base));
    env::join_paths(paths).unwrap_or_else(|_| base.into())
}

fn resolve_on_path(host_path: &OsStr, program: &str) -> Option<PathBuf> {
    env::split_paths(host_path)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
        .map(|candidate| candidate.canonicalize().unwrap_or(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// The install root above a rustup toolchain's `bin`: `.../toolchains/<name>`
/// for `.../toolchains/<name>/bin/cargo`. Anything else is not a rustup
/// layout and yields `None`.
fn rust_toolchain_root(binary: &Path) -> Option<PathBuf> {
    let bin = binary.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    let root = bin.parent()?;
    let toolchains = root.parent()?;
    (toolchains.file_name()? == "toolchains").then(|| root.to_path_buf())
}

fn push_dir(dirs: &mut Vec<PathBuf>, candidate: PathBuf, home: &Path, state_roots: &[PathBuf]) {
    if dirs.len() >= MAX_TOOLCHAIN_DIRS {
        return;
    }
    let Ok(canonical) = candidate.canonicalize() else {
        return;
    };
    if !canonical.is_dir() || canonical == home {
        return;
    }
    if state_roots
        .iter()
        .any(|root| canonical == *root || canonical.starts_with(root))
    {
        return;
    }
    if canonical.components().any(|component| {
        FORBIDDEN_COMPONENTS
            .iter()
            .any(|name| component.as_os_str() == OsStr::new(name))
    }) {
        return;
    }
    if !dirs.contains(&canonical) {
        dirs.push(canonical);
    }
}

#[cfg(all(test, unix))]
#[path = "toolchain_tests.rs"]
mod toolchain_tests;
