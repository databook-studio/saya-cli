use super::*;
use saya_harness::workspace::Workspace;

/// The commit-time content guard: an observed digest routes the commit
/// through `Workspace::patch_range_guarded`, so a same-size change
/// between the caller's read and the commit is refused as the tool's
/// typed moved-anchor error — never a splice against stale content.
/// Focused here (not through the tool executor) because no public API
/// can change a file deterministically between the tool's own read and
/// its commit inside one call.
#[test]
fn a_stale_observed_digest_refuses_the_commit_as_a_moved_anchor() {
    let root = std::env::temp_dir().join(format!("saya-splice-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("sandbox directory must create");
    let workspace = Workspace::open(&root).expect("workspace must open");
    workspace
        .write("notes.md", b"hello world")
        .expect("seed must write");
    let observed = workspace
        .read("notes.md", 0)
        .expect("seed must read")
        .digest;
    // Same-size (11 bytes) overwrite between the observation and the
    // commit: a length guard alone cannot see this.
    workspace
        .write("notes.md", b"HELLO WORLD")
        .expect("overwrite must write");
    let error = commit_splice(&workspace, "notes.md", 0..5, 11, Some(&observed), b"WORLD")
        .expect_err("a stale observed digest must refuse the commit");
    match error {
        ToolError::WorkspaceEditMoved {
            path,
            expected_size,
            current_size,
            expected_digest,
            ..
        } => {
            assert_eq!(path, "notes.md");
            assert_eq!(expected_size, Some(11));
            assert_eq!(current_size, 11);
            assert_eq!(expected_digest, Some(observed));
        }
        other => panic!("expected the typed moved-anchor refusal, got: {other}"),
    }
    assert_eq!(
        std::fs::read(root.join("notes.md")).expect("file must survive"),
        b"HELLO WORLD",
        "a refused commit leaves the file byte-identical"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Losing a creation race never clobbers: the append half's create
/// helper is create-if-absent, so a file that appeared between the
/// probe and the create refuses typed (the offset mismatch the model
/// re-anchors from, naming the winner's state) and the winner's bytes
/// are left untouched — never an unconditional replace. Focused here
/// (not through the tool executor) because `write_new` is
/// `pub(super)`: no public API can make a file appear between the
/// tool's own probe and its create inside one call.
#[test]
fn losing_a_creation_race_never_clobbers() {
    let root = std::env::temp_dir().join(format!("saya-create-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("sandbox directory must create");
    let workspace = Workspace::open(&root).expect("workspace must open");
    let winner = b"the racing creator's bytes";
    workspace
        .write("notes.md", winner)
        .expect("seed must write");
    let error = write_new(&workspace, "notes.md", b"loser bytes")
        .expect_err("creating over an existing file must refuse");
    match error {
        ToolError::WorkspaceAppendOffset {
            path,
            expected_offset,
            current_size,
            current_digest,
        } => {
            assert_eq!(path, "notes.md");
            assert_eq!(expected_offset, 0);
            assert_eq!(current_size, winner.len() as u64);
            assert_eq!(current_digest.len(), 64);
        }
        other => panic!("expected the typed creation-race refusal, got: {other}"),
    }
    assert_eq!(
        std::fs::read(root.join("notes.md")).expect("file must survive"),
        winner,
        "the losing creation leaves the winner's bytes untouched"
    );
    let _ = std::fs::remove_dir_all(&root);
}
