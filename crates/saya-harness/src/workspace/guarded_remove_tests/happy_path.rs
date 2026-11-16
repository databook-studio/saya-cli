//! The committed removals: a matching digest deletes exactly its file and
//! keeps the parent directory in place, and the freed path accepts a fresh
//! `create_new`.

use std::fs;

use super::{Sandbox, bytes_of};

/// The happy path: a removal whose fresh re-read hashes to the observed
/// digest deletes exactly that file, leaves its parent directory in place,
/// and leaves no temp file behind.
#[test]
fn remove_guarded_removes_a_matching_file_and_keeps_the_parent() {
    let sandbox = Sandbox::new("rm-ok");
    sandbox.ws.write("dir/victim.txt", b"hello world").unwrap();
    let observed = sandbox.ws.read("dir/victim.txt", 4096).unwrap();

    sandbox
        .ws
        .remove_guarded("dir/victim.txt", &observed.digest)
        .expect("removal with the observed digest must commit");
    assert!(!sandbox.root().join("dir/victim.txt").exists());
    assert!(
        sandbox.root().join("dir").is_dir(),
        "the parent directory must remain"
    );
    let strays: Vec<String> = fs::read_dir(sandbox.root().join("dir"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    assert!(strays.is_empty(), "strays after removal: {strays:?}");
}

/// Recovery's undo shape: after a successful removal the path is free again,
/// so a `create_new` on it succeeds whole.
#[test]
fn remove_guarded_allows_create_new_afterwards() {
    let sandbox = Sandbox::new("rm-recreate");
    sandbox.ws.write("victim.txt", b"hello").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();

    sandbox
        .ws
        .remove_guarded("victim.txt", &observed.digest)
        .expect("removal with the observed digest must commit");
    assert!(!sandbox.root().join("victim.txt").exists());

    sandbox
        .ws
        .create_new("victim.txt", b"fresh")
        .expect("create after removal must succeed");
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"fresh");
}
