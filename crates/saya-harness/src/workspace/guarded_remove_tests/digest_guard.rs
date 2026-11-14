//! The digest guard's refusals: a wrong observed digest and a file that
//! changed after observation both refuse `ContentChanged` with the newer
//! bytes intact and no temp file left behind.

use super::{Sandbox, bytes_of, stray_names};

use crate::HarnessError;

/// A wrong digest refuses with `ContentChanged` naming the expected and the
/// actual digests, and the file stays byte-identical — nothing is deleted.
#[test]
fn remove_guarded_refuses_a_wrong_digest_and_preserves_the_bytes() {
    let sandbox = Sandbox::new("rm-wrong");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    sandbox
        .ws
        .write("sub/other.txt", b"different content")
        .unwrap();
    let other_digest = sandbox.ws.read("sub/other.txt", 4096).unwrap().digest;
    let before = bytes_of(&sandbox.ws, "victim.txt");

    let error = sandbox
        .ws
        .remove_guarded("victim.txt", &other_digest)
        .expect_err("a wrong digest must refuse");
    assert!(
        matches!(
            &error,
            HarnessError::ContentChanged { path, expected, current }
                if path == "victim.txt" && *expected == other_digest && !current.is_empty()
        ),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);
    let strays = stray_names(&sandbox);
    assert!(strays.is_empty(), "stray files after refusal: {strays:?}");
}

/// The stale removal a size check cannot see: the file changed between the
/// observation and the call, so only the digest refuses. The newer bytes
/// stay and no temp file is created anywhere.
#[test]
fn remove_guarded_refuses_content_changed_after_observation() {
    let sandbox = Sandbox::new("rm-stale");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();
    sandbox.ws.write("victim.txt", b"HELLO WORLD").unwrap();

    let error = sandbox
        .ws
        .remove_guarded("victim.txt", &observed.digest)
        .expect_err("changed content must refuse");
    assert!(
        matches!(
            &error,
            HarnessError::ContentChanged { path, expected, current }
                if path == "victim.txt" && *expected == observed.digest && !current.is_empty()
        ),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"HELLO WORLD");
    let strays = stray_names(&sandbox);
    assert!(strays.is_empty(), "stray files after refusal: {strays:?}");
}
