//! The manifest battery: names/sizes/digests for episode briefs — never bulk
//! contents — with its bounds enforced as typed errors and hygiene content
//! skipped.

use std::{fs, path::PathBuf};

use saya_harness::HarnessError;
use saya_harness::workspace::{Workspace, manifest};

const CONTENT: &[u8] = b"inside-sentinel";

struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-manifest-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).unwrap();
        let ws = Workspace::open(&outer.join("ws")).unwrap();
        Self { outer, ws }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

#[test]
fn manifest_lists_relative_paths_sizes_and_known_digests() {
    let sandbox = Sandbox::new("entries");
    sandbox.ws.write("report.md", CONTENT).unwrap();
    sandbox.ws.write("nested/data.csv", CONTENT).unwrap();

    let entries = manifest::build(&sandbox.ws, 100, 1 << 20).unwrap();
    let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["nested/data.csv", "report.md"],
        "sorted, slash-separated"
    );

    for entry in &entries {
        assert_eq!(entry.size, CONTENT.len() as u64);
        assert_eq!(
            entry.digest, "b568f317d963edff551663a3b460d49899fa97a2d313bb50078916da9e74d15f",
            "digest must be the known sha256 of the content"
        );
    }
}

#[test]
fn empty_workspaces_manifest_to_nothing() {
    let sandbox = Sandbox::new("empty");
    let entries = manifest::build(&sandbox.ws, 100, 1 << 20).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn manifest_is_deterministic_across_calls() {
    let sandbox = Sandbox::new("deterministic");
    sandbox.ws.write("b.txt", CONTENT).unwrap();
    sandbox.ws.write("a.txt", CONTENT).unwrap();
    sandbox.ws.write("dir/c.txt", CONTENT).unwrap();

    let first = manifest::build(&sandbox.ws, 100, 1 << 20).unwrap();
    let second = manifest::build(&sandbox.ws, 100, 1 << 20).unwrap();
    assert_eq!(first, second);
}

#[test]
fn file_count_bounds_refuse_floods() {
    let sandbox = Sandbox::new("count-flood");
    for i in 0..6 {
        sandbox.ws.write(&format!("f{i}.txt"), CONTENT).unwrap();
    }
    let entries = manifest::build(&sandbox.ws, 6, 1 << 20).unwrap();
    assert_eq!(entries.len(), 6);

    let error = manifest::build(&sandbox.ws, 5, 1 << 20).expect_err("flood must refuse");
    assert!(
        matches!(error, HarnessError::BoundsExceeded { .. }),
        "{error:?}"
    );
}

#[test]
fn per_file_digest_bounds_refuse_huge_files() {
    let sandbox = Sandbox::new("digest-cap");
    let huge = vec![b'x'; 1 << 20];
    sandbox.ws.write("huge.bin", &huge).unwrap();

    let error = manifest::build(&sandbox.ws, 100, 1024).expect_err("must refuse");
    assert!(
        matches!(error, HarnessError::BoundsExceeded { .. }),
        "{error:?}"
    );

    let entries = manifest::build(&sandbox.ws, 100, 1 << 20).unwrap();
    assert_eq!(entries.len(), 1);
}

#[test]
#[cfg(unix)]
fn symlinked_entries_are_never_digested() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("symlink");
    fs::write(sandbox.outer.join("outside.txt"), b"outside").unwrap();
    symlink(
        sandbox.outer.join("outside.txt"),
        sandbox.ws.root().join("leak.txt"),
    )
    .unwrap();

    let entries = manifest::build(&sandbox.ws, 100, 1 << 20).unwrap();
    assert!(
        entries.iter().all(|entry| entry.path != "leak.txt"),
        "a link must never enter the brief: {entries:?}"
    );
    // Nothing outside was read or digested through the link.
    assert!(fs::read(sandbox.outer.join("outside.txt")).is_ok());
}
