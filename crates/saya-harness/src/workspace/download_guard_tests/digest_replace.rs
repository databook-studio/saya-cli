//! The digest-replace precondition of the destination guard: a matching
//! digest replaces, a stale or wrong-shape digest refuses untouched, a
//! digest never authorizes creation, and an over-limit destination refuses
//! without being hashed.

use std::fs;

use crate::HarnessError;

use super::super::DESTINATION_DIGEST_MAX_BYTES;
use super::{DEST, EMPTY_DIGEST, PART, Sandbox};

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
