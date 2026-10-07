//! Adversarial tests for the download promotion's destination guard: the
//! create-only default, the digest-replace precondition, and the refusals
//! that leave the destination and the completed part untouched. Real temp
//! workspaces and real files throughout — the guard is exercised through
//! `Workspace::promote_download`, the dispatcher the download machinery
//! calls, never through a mock.

use std::fs;

use crate::HarnessError;
use crate::workspace::Workspace;

use super::DESTINATION_DIGEST_MAX_BYTES;

const DEST: &str = "downloads/doc.bin";
const PART: &str = "downloads/doc.bin.saya-part";

/// SHA-256 of the empty byte string — a digest no non-empty fixture hashes
/// to.
const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

struct Sandbox {
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("saya-dl-guard-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let ws = Workspace::open(&root).expect("workspace root must open");
        Self { ws }
    }

    fn root(&self) -> &std::path::Path {
        self.ws.root()
    }

    /// Seeds the completed part with the content the promotion must land at
    /// the destination.
    fn seed_part(&self, bytes: &[u8]) {
        self.ws.write(PART, bytes).unwrap();
    }

    fn part_bytes(&self) -> Option<Vec<u8>> {
        fs::read(self.root().join(PART)).ok()
    }

    fn dest_bytes(&self) -> Option<Vec<u8>> {
        fs::read(self.root().join(DEST)).ok()
    }

    /// The destination's whole-content digest as a caller would have
    /// observed it through a contained read.
    fn observed_dest_digest(&self) -> String {
        self.ws.read(DEST, 4096).unwrap().digest
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.root());
    }
}

/// The create-only default: with no digest, an existing destination refuses
/// `AlreadyExists`, and destination and part both stay byte-identical.
#[test]
fn an_existing_destination_refuses_creation_and_leaves_both_sides_untouched() {
    let sandbox = Sandbox::new("create-refused");
    sandbox.seed_part(b"fresh download bytes");
    fs::write(sandbox.root().join(DEST), b"existing content").unwrap();

    let error = sandbox
        .ws
        .promote_download(PART, DEST, None)
        .expect_err("a create over an existing destination must refuse");
    assert!(
        matches!(&error, HarnessError::AlreadyExists { path } if path == DEST),
        "{error:?}"
    );
    assert_eq!(
        sandbox.dest_bytes().as_deref(),
        Some(b"existing content".as_slice()),
        "the existing destination is untouched"
    );
    assert_eq!(
        sandbox.part_bytes().as_deref(),
        Some(b"fresh download bytes".as_slice()),
        "the part survives the refusal"
    );
}

/// A stale digest refuses `ContentChanged`: the destination no longer holds
/// what the caller observed, so the rename never happens.
#[test]
fn a_stale_digest_refuses_replacement_and_leaves_both_sides_untouched() {
    let sandbox = Sandbox::new("stale-digest");
    sandbox.seed_part(b"fresh download bytes");
    fs::write(sandbox.root().join(DEST), b"observed content").unwrap();
    let observed = sandbox.observed_dest_digest();
    fs::write(sandbox.root().join(DEST), b"changed content").unwrap();

    let error = sandbox
        .ws
        .promote_download(PART, DEST, Some(&observed))
        .expect_err("a digest the destination no longer hashes to must refuse");
    assert!(
        matches!(&error, HarnessError::ContentChanged { path, expected, current }
            if path == DEST && *expected == observed && !current.is_empty()),
        "{error:?}"
    );
    assert_eq!(
        sandbox.dest_bytes().as_deref(),
        Some(b"changed content".as_slice()),
        "the changed destination is untouched"
    );
    assert_eq!(
        sandbox.part_bytes().as_deref(),
        Some(b"fresh download bytes".as_slice()),
        "the part survives the refusal"
    );
}

/// The happy replace: with the digest of the destination's current bytes,
/// the part replaces the destination and the rename consumes the part.
#[test]
fn a_matching_digest_replaces_the_destination_and_consumes_the_part() {
    let sandbox = Sandbox::new("replace-ok");
    sandbox.seed_part(b"fresh download bytes");
    fs::write(sandbox.root().join(DEST), b"stale old bytes").unwrap();
    let observed = sandbox.observed_dest_digest();

    sandbox
        .ws
        .promote_download(PART, DEST, Some(&observed))
        .expect("replacement with the observed digest must commit");
    assert_eq!(
        sandbox.dest_bytes().as_deref(),
        Some(b"fresh download bytes".as_slice())
    );
    assert!(
        sandbox.part_bytes().is_none(),
        "the rename consumed the part"
    );
}

/// Without a digest, an absent destination is created.
#[test]
fn an_absent_destination_is_created_without_a_digest() {
    let sandbox = Sandbox::new("create-ok");
    sandbox.seed_part(b"fresh download bytes");

    sandbox
        .ws
        .promote_download(PART, DEST, None)
        .expect("an absent destination is a create");
    assert_eq!(
        sandbox.dest_bytes().as_deref(),
        Some(b"fresh download bytes".as_slice())
    );
    assert!(
        sandbox.part_bytes().is_none(),
        "the rename consumed the part"
    );
}

/// A digest grants no authority to create: on a missing destination it is a
/// `NotFound` refusal — naming no digest — and the part stays.
#[test]
fn a_digest_on_an_absent_destination_refuses_not_found() {
    let sandbox = Sandbox::new("digest-missing");
    sandbox.seed_part(b"fresh download bytes");

    let error = sandbox
        .ws
        .promote_download(PART, DEST, Some(EMPTY_DIGEST))
        .expect_err("a digest on a missing destination must refuse, not create");
    assert!(
        matches!(&error, HarnessError::NotFound { path } if path == DEST),
        "{error:?}"
    );
    assert!(sandbox.dest_bytes().is_none(), "nothing was created");
    assert_eq!(
        sandbox.part_bytes().as_deref(),
        Some(b"fresh download bytes".as_slice()),
        "the part survives the refusal"
    );
}

/// A directory at the destination refuses both ways: `AlreadyExists` under
/// the create-only default, `NotRegularFile` against a digest — the part
/// survives both.
#[test]
fn a_directory_at_the_destination_refuses_both_ways() {
    let sandbox = Sandbox::new("dir-dest");
    sandbox.seed_part(b"fresh download bytes");
    fs::create_dir_all(sandbox.root().join(DEST)).unwrap();

    let error = sandbox
        .ws
        .promote_download(PART, DEST, None)
        .expect_err("a create over a directory must refuse");
    assert!(
        matches!(&error, HarnessError::AlreadyExists { path } if path == DEST),
        "{error:?}"
    );
    assert!(sandbox.root().join(DEST).is_dir(), "the directory stays");
    assert!(sandbox.part_bytes().is_some(), "the part survives");

    let error = sandbox
        .ws
        .promote_download(PART, DEST, Some(EMPTY_DIGEST))
        .expect_err("a digest cannot authorize replacing a directory");
    assert!(
        matches!(&error, HarnessError::NotRegularFile { path } if path == DEST),
        "{error:?}"
    );
    assert!(sandbox.root().join(DEST).is_dir(), "the directory stays");
    assert!(sandbox.part_bytes().is_some(), "the part survives");
}

/// Digest text that is not a digest simply mismatches — typed
/// `ContentChanged`, the same shape the guarded write and patch carry for
/// wrong-shape preconditions — and both sides stay untouched.
#[test]
fn an_invalid_digest_text_refuses_typed_and_touches_nothing() {
    let sandbox = Sandbox::new("bad-digest");
    sandbox.seed_part(b"fresh download bytes");
    fs::write(sandbox.root().join(DEST), b"existing content").unwrap();

    let error = sandbox
        .ws
        .promote_download(PART, DEST, Some("not-a-digest"))
        .expect_err("digest text that is not a digest must refuse");
    assert!(
        matches!(&error, HarnessError::ContentChanged { path, .. } if path == DEST),
        "{error:?}"
    );
    assert_eq!(
        sandbox.dest_bytes().as_deref(),
        Some(b"existing content".as_slice()),
        "the existing destination is untouched"
    );
    assert!(sandbox.part_bytes().is_some(), "the part survives");
}

/// A destination over the download file limit refuses without being hashed:
/// `BoundsExceeded` naming the scanned size and the bound, part untouched.
#[test]
fn an_oversized_destination_refuses_without_being_hashed() {
    let sandbox = Sandbox::new("oversized");
    sandbox.seed_part(b"fresh download bytes");
    let oversized = fs::File::create(sandbox.root().join(DEST)).unwrap();
    oversized.set_len(DESTINATION_DIGEST_MAX_BYTES + 1).unwrap();
    drop(oversized);

    let error = sandbox
        .ws
        .promote_download(PART, DEST, Some(EMPTY_DIGEST))
        .expect_err("an over-limit destination must refuse");
    assert!(
        matches!(&error, HarnessError::BoundsExceeded { path, found, max }
            if path == DEST && *found == DESTINATION_DIGEST_MAX_BYTES + 1
                && *max == DESTINATION_DIGEST_MAX_BYTES),
        "{error:?}"
    );
    assert_eq!(
        sandbox.part_bytes().as_deref(),
        Some(b"fresh download bytes".as_slice()),
        "the part survives the refusal"
    );
}
