//! Refusals before any filesystem contact: escaping or invalid arguments,
//! and a target over the whole-file I/O bound.

use std::fs;

use super::{EMPTY_DIGEST, Sandbox, bytes_of};

use crate::HarnessError;
use crate::workspace::MAX_IO_BYTES;

/// Traversal and absolute forms refuse before any filesystem contact, and a
/// real file beside them is untouched.
#[test]
fn remove_guarded_refuses_escaping_and_invalid_arguments() {
    let sandbox = Sandbox::new("rm-escape");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    let before = bytes_of(&sandbox.ws, "victim.txt");

    for argument in ["../outside.txt", "/etc/passwd"] {
        let error = sandbox
            .ws
            .remove_guarded(argument, EMPTY_DIGEST)
            .expect_err("escaping argument must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "{argument} → {error:?}"
        );
    }
    let error = sandbox
        .ws
        .remove_guarded("", EMPTY_DIGEST)
        .expect_err("empty argument must refuse");
    assert!(
        matches!(error, HarnessError::InvalidPath { .. }),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);
}

/// A file over the whole-file I/O bound refuses before anything is read or
/// removed: the typed bound names the size and the cap, and the file keeps
/// its bytes.
#[test]
fn remove_guarded_over_the_io_bound_is_refused_whole() {
    let sandbox = Sandbox::new("rm-bound");
    let huge = vec![b'x'; MAX_IO_BYTES + 1];
    fs::create_dir_all(sandbox.root().join("big")).unwrap();
    fs::write(sandbox.root().join("big/huge.bin"), &huge).unwrap();

    let error = sandbox
        .ws
        .remove_guarded("big/huge.bin", EMPTY_DIGEST)
        .expect_err("over-bound target must refuse");
    assert!(
        matches!(
            error,
            HarnessError::BoundsExceeded { found, max, .. }
                if found == huge.len() as u64 && max == MAX_IO_BYTES as u64
        ),
        "{error:?}"
    );
    assert_eq!(
        fs::read(sandbox.root().join("big/huge.bin")).unwrap().len(),
        huge.len()
    );
}
