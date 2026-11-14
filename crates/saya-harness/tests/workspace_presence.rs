//! The bounded presence check's public contract: the walk the searches use,
//! stopped at the first match, under a named entry bound and deadline.
//!
//! The check decides whether a surface may offer a tool that only a notebook
//! makes useful, so what it must never do is answer "no" from a walk it gave
//! up on. `complete` is what separates the two, and these tests pin both.

use saya_harness::workspace::{PRESENCE_MAX_ENTRIES, PresenceBounds, Workspace};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

struct Fixture {
    outer: PathBuf,
    workspace: Workspace,
}

impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let outer = std::env::temp_dir().join(format!(
            "saya-presence-{label}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&outer).unwrap();
        fs::create_dir(outer.join("ws")).unwrap();
        let workspace = Workspace::open(&outer.join("ws")).unwrap();
        Self { outer, workspace }
    }

    fn put(&self, path: &str, bytes: &[u8]) {
        let target = self.workspace.root().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, bytes).unwrap();
    }

    fn present(&self) -> saya_harness::workspace::Presence {
        self.workspace
            .holds_file_extension("ipynb", PresenceBounds::default())
    }

    fn present_under(&self, bounds: PresenceBounds) -> saya_harness::workspace::Presence {
        self.workspace.holds_file_extension("ipynb", bounds)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("retained presence fixture: {}", self.outer.display());
        } else {
            fs::remove_dir_all(&self.outer).unwrap();
        }
    }
}

/// A notebook anywhere the walk reaches answers yes — nested, and whatever
/// case its extension carries.
#[test]
fn a_notebook_anywhere_in_the_tree_is_found() {
    let fixture = Fixture::new("found");
    fixture.put("src/deep/notes.txt", b"not a notebook\n");
    fixture.put("src/deep/analysis.ipynb", b"{}");
    let found = fixture.present();
    assert!(found.found, "{found:?}");
    assert!(found.complete, "a match is a definite answer: {found:?}");

    let upper = Fixture::new("found-upper");
    upper.put("REPORT.IPYNB", b"{}");
    assert!(
        upper.present().found,
        "the extension compares case-insensitively"
    );
}

/// A workspace with no notebook answers no, and answers it completely: the
/// walk finished, so the caller may treat it as a definite "not here".
#[test]
fn a_workspace_without_a_notebook_answers_a_complete_no() {
    let fixture = Fixture::new("absent");
    fixture.put("src/main.rs", b"fn main() {}\n");
    fixture.put("README.md", b"# readme\n");
    let absent = fixture.present();
    assert!(!absent.found, "{absent:?}");
    assert!(absent.complete, "{absent:?}");
}

/// A directory the workspace's own `.gitignore` hides is not descended into
/// and its files are not counted — the check reuses the searches' ignore
/// policy rather than a rule of its own.
#[test]
fn an_ignored_notebook_is_not_counted() {
    let fixture = Fixture::new("ignored");
    fixture.put(".gitignore", b"*.ipynb\n");
    fixture.put("build/analysis.ipynb", b"{}");
    let ignored = fixture.present();
    assert!(!ignored.found, "{ignored:?}");
    assert!(ignored.complete, "{ignored:?}");
}

/// A symlink named like a notebook is not one: the walk refuses links, so a
/// link pointing at a notebook outside the workspace counts for nothing.
#[cfg(unix)]
#[test]
fn a_symlinked_notebook_is_not_counted() {
    let fixture = Fixture::new("symlink");
    let outside = fixture.outer.join("outside.ipynb");
    fs::write(&outside, b"{}").unwrap();
    std::os::unix::fs::symlink(&outside, fixture.workspace.root().join("link.ipynb")).unwrap();
    let linked = fixture.present();
    assert!(!linked.found, "{linked:?}");
    assert!(linked.complete, "{linked:?}");
}

/// A tree larger than the entry bound, with no notebook in it, stops at the
/// bound: the answer is incomplete, which the caller reads as "do not offer
/// the tool", never as a definite no.
#[test]
fn a_tree_over_the_entry_bound_stops_at_the_bound() {
    let fixture = Fixture::new("bound");
    let bounds = PresenceBounds {
        entries: 4,
        ..PresenceBounds::default()
    };
    for index in 0..32 {
        fixture.put(&format!("file-{index}.txt"), b"x");
    }
    let stopped = fixture.present_under(bounds);
    assert!(!stopped.found, "{stopped:?}");
    assert!(!stopped.complete, "{stopped:?}");
    assert_eq!(
        stopped.entries_visited, bounds.entries,
        "the walk must stop exactly at the bound it was given: {stopped:?}"
    );

    // The same tree answers completely once the bound can hold it — the
    // incomplete answer above is the bound, not the tree.
    let whole = fixture.present_under(PresenceBounds {
        entries: PRESENCE_MAX_ENTRIES,
        ..PresenceBounds::default()
    });
    assert!(!whole.found && whole.complete, "{whole:?}");
}

/// A deadline already past stops the check the same way the entry bound does:
/// no answer, and no match invented from a walk that never ran.
#[test]
fn a_spent_deadline_stops_the_check() {
    let fixture = Fixture::new("deadline");
    fixture.put("analysis.ipynb", b"{}");
    let expired = fixture.present_under(PresenceBounds {
        timeout: Duration::ZERO,
        ..PresenceBounds::default()
    });
    assert!(!expired.found, "{expired:?}");
    assert!(!expired.complete, "{expired:?}");
}
