//! Refusals for targets that are not regular files: a missing final
//! component is the typed `NotFound`, and a directory at the path is
//! `NotRegularFile` with its content untouched.

use super::{EMPTY_DIGEST, Sandbox, bytes_of};

use crate::HarnessError;

/// Removal names an existing file only: a missing final component is the
/// typed `NotFound` — removal of nothing is not success — and the refusal
/// creates nothing.
#[test]
fn remove_guarded_refuses_a_missing_file_with_not_found() {
    let sandbox = Sandbox::new("rm-missing");

    let error = sandbox
        .ws
        .remove_guarded("absent.txt", EMPTY_DIGEST)
        .expect_err("missing target must refuse");
    assert!(
        matches!(&error, HarnessError::NotFound { path } if path == "absent.txt"),
        "{error:?}"
    );
    assert!(!sandbox.root().join("absent.txt").exists());

    let error = sandbox
        .ws
        .remove_guarded("new/dir/file.txt", EMPTY_DIGEST)
        .expect_err("missing parents must refuse too");
    assert!(
        matches!(&error, HarnessError::NotFound { path } if path == "new/dir/file.txt"),
        "{error:?}"
    );
    assert!(!sandbox.root().join("new").exists());
}

/// A directory at the path refuses as `NotRegularFile`, and the directory's
/// own content is untouched.
#[test]
fn remove_guarded_refuses_a_directory_target() {
    let sandbox = Sandbox::new("rm-dir");
    sandbox.ws.write("sub/nested.txt", b"keep me").unwrap();

    let error = sandbox
        .ws
        .remove_guarded("sub", EMPTY_DIGEST)
        .expect_err("directory target must refuse");
    assert!(
        matches!(error, HarnessError::NotRegularFile { .. }),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "sub/nested.txt"), b"keep me");
}
