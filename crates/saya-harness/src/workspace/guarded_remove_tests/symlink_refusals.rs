//! Symlink refusals, gated on unix: neither a symlinked final component nor
//! a symlinked parent is followed, and nothing behind the link is removed.

use std::fs;

use super::{EMPTY_DIGEST, OUTSIDE, Sandbox};

use crate::HarnessError;

/// A symlink at the final component is refused without being followed or
/// removed: the target keeps its exact bytes and the link itself survives.
#[cfg(unix)]
#[test]
fn remove_guarded_refuses_a_final_component_symlink_and_spares_the_target() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("rm-link");
    let outside = sandbox.outside_path("outside.txt");
    fs::write(&outside, OUTSIDE).unwrap();
    symlink(&outside, sandbox.root().join("link")).unwrap();

    let error = sandbox
        .ws
        .remove_guarded("link", EMPTY_DIGEST)
        .expect_err("symlink target must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    assert_eq!(fs::read(&outside).unwrap(), OUTSIDE);
    assert!(
        sandbox
            .root()
            .join("link")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link itself must not have been removed"
    );
}

/// A symlinked parent component is refused by the walk before the final
/// component is ever touched: nothing behind the link is removed.
#[cfg(unix)]
#[test]
fn remove_guarded_refuses_a_symlinked_parent_component() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("rm-linkdir");
    let outside_dir = sandbox.outer.join("outside-dir");
    fs::create_dir_all(&outside_dir).unwrap();
    fs::write(outside_dir.join("inside.txt"), OUTSIDE).unwrap();
    symlink(&outside_dir, sandbox.root().join("linkdir")).unwrap();

    let error = sandbox
        .ws
        .remove_guarded("linkdir/inside.txt", EMPTY_DIGEST)
        .expect_err("symlinked parent must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    assert_eq!(fs::read(outside_dir.join("inside.txt")).unwrap(), OUTSIDE);
}
