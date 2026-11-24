//! The sweep's guard: every source and test file is named by a declared
//! module, and every snapshot names a test that still exists.
//!
//! X1 deleted three trunks and left files behind that nothing declares any
//! more — a test module whose `mod` line went with the code it drove, a
//! fixture beside it. Nothing fails when one of those rots, because the file
//! is simply never compiled. This test is what fails instead.
//!
//! It reads the tree, not the binary: a file nobody declares is invisible to
//! `cargo`, so no other test can catch it. The rule is deliberately the
//! textual one — a `mod <stem>;` somewhere in the same crate, or an explicit
//! `#[path]` / `include!` string — because that is the whole of what makes a
//! file part of the build.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The workspace root: `crates/saya-cli` → up two.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels under the workspace root")
        .to_path_buf()
}

/// Every file under `dir` whose extension is `extension`.
fn files_with_extension(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let skip = matches!(
            entry.file_name().to_str(),
            Some("target" | ".git" | "__pycache__")
        );
        if path.is_dir() {
            if !skip {
                files_with_extension(&path, extension, out);
            }
        } else if path.extension().is_some_and(|ext| ext == extension) {
            out.push(path);
        }
    }
}

/// Every module name a line declares out of line (`mod name;`), whether or
/// not it sits inside an inline module body.
fn declared_module(line: &str) -> Option<&str> {
    let rest = line.trim().split_once("mod ").map(|(_, rest)| rest)?;
    let name = rest.trim_end().strip_suffix(';')?.trim();
    let plain = !name.is_empty() && name.chars().all(|c| c == '_' || c.is_alphanumeric());
    plain.then_some(name)
}

/// The quoted argument of every `#[path = "…"]`, `include!("…")`,
/// `include_str!("…")` and `include_bytes!("…")` in `text` — including the
/// ones whose argument sits on the next line.
fn quoted_paths(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for marker in ["#[path = ", "include!(", "include_str!(", "include_bytes!("] {
        for (index, _) in text.match_indices(marker) {
            let rest = &text[index + marker.len()..];
            let Some(start) = rest.find('"') else {
                continue;
            };
            let rest = &rest[start + 1..];
            if let Some(end) = rest.find('"') {
                found.push(&rest[..end]);
            }
        }
    }
    found
}

/// What one crate declares: the module names it compiles, and the files its
/// `#[path]` / `include!` strings name.
struct Declarations {
    names: BTreeSet<String>,
    paths: Vec<PathBuf>,
}

impl Declarations {
    fn of(crate_dir: &Path) -> Self {
        let mut sources = Vec::new();
        for part in ["src", "tests"] {
            files_with_extension(&crate_dir.join(part), "rs", &mut sources);
        }
        let mut names = BTreeSet::new();
        let mut paths = Vec::new();
        for file in &sources {
            let text = fs::read_to_string(file).unwrap_or_default();
            for name in text.lines().filter_map(declared_module) {
                names.insert(name.to_owned());
            }
            let dir = file.parent().expect("a file has a parent");
            for quoted in quoted_paths(&text) {
                paths.push(dir.join(quoted).canonicalize().unwrap_or_default());
            }
        }
        Self { names, paths }
    }

    /// Whether this file is part of the build: a crate root, a `mod.rs` whose
    /// directory is declared, a module-named file, or a quoted path.
    fn declares(&self, file: &Path) -> bool {
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if matches!(stem, "lib" | "main") {
            return true;
        }
        // A `tests/*.rs` file is its own test target, and a `tests/<dir>/main.rs`
        // is one when no `tests/<dir>.rs` shadows it: both compile without a
        // `mod` line anywhere.
        let dir = file.parent().expect("a file has a parent");
        if dir.file_name().is_some_and(|name| name == "tests") {
            return true;
        }
        let sibling = dir.with_extension("rs");
        if file.file_name().is_some_and(|name| name == "main.rs") && !sibling.is_file() {
            return true;
        }
        let named = if stem == "mod" {
            file.parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .is_some_and(|dir| self.names.contains(dir))
        } else {
            self.names.contains(stem)
        };
        named
            || self
                .paths
                .contains(&file.canonicalize().unwrap_or_default())
    }
}

#[test]
fn every_source_and_test_file_is_named_by_a_declared_module() {
    let root = workspace_root();
    // This crate only: it is the one X1–X3 deleted files from. The harness's
    // own uncompiled `budgeted_child_scheduler/` predates that campaign and is
    // reported, not fixed, here.
    let crate_dir = root.join("crates/saya-cli");
    let declarations = Declarations::of(&crate_dir);
    let mut files = Vec::new();
    for part in ["src", "tests"] {
        files_with_extension(&crate_dir.join(part), "rs", &mut files);
    }
    let mut orphans: Vec<String> = files
        .into_iter()
        .filter(|file| !declarations.declares(file))
        .map(|file| {
            file.strip_prefix(&root)
                .unwrap_or(&file)
                .display()
                .to_string()
        })
        .collect();
    orphans.sort();
    assert!(
        orphans.is_empty(),
        "{} file(s) no declared module names: {orphans:#?}",
        orphans.len()
    );
}

#[test]
fn every_snapshot_names_a_test_that_still_exists() {
    let root = workspace_root();
    let mut snapshots = Vec::new();
    files_with_extension(&root.join("crates"), "snap", &mut snapshots);
    let mut sources = Vec::new();
    files_with_extension(&root.join("crates"), "rs", &mut sources);
    let mut text = String::new();
    for file in &sources {
        text.push_str(&fs::read_to_string(file).unwrap_or_default());
        text.push('\n');
    }
    let mut stale = Vec::new();
    for snapshot in &snapshots {
        let name = snapshot
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        // insta names a snapshot `<crate>__<module path>__<test>`.
        let test = name.rsplit("__").next().unwrap_or(name);
        let declared = ["(", "()", "<"]
            .iter()
            .any(|suffix| text.contains(&format!("fn {test}{suffix}")));
        if !declared {
            stale.push(
                snapshot
                    .strip_prefix(&root)
                    .unwrap_or(snapshot)
                    .display()
                    .to_string(),
            );
        }
    }
    stale.sort();
    assert!(
        stale.is_empty(),
        "{} snapshot(s) name no existing test: {stale:#?}",
        stale.len()
    );
}
