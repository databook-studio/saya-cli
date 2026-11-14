//! The create-only default of the destination guard: with no digest, an
//! absent destination is created, and an existing destination — file or
//! directory — refuses untouched.

use std::fs;

use crate::HarnessError;

use super::{DEST, EMPTY_DIGEST, PART, Sandbox};

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
