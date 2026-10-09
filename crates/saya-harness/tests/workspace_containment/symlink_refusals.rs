use super::*;

#[cfg(unix)]
#[test]
fn symlink_as_final_component_is_refused_not_followed() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("symlink-final");
    sandbox.plant();

    let outside = sandbox.outside_path("outside.txt");
    symlink(&outside, sandbox.root().join("link")).unwrap();

    let error = sandbox.ws.read("link", 4096).expect_err("link must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );

    let error = sandbox
        .ws
        .write("link", b"overwrite-through-link")
        .expect_err("writing through a link must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    assert_eq!(
        fs::read(&outside).unwrap(),
        OUTSIDE,
        "outside file untouched"
    );
}

#[cfg(unix)]
#[test]
fn symlink_chains_and_parent_components_are_refused() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("symlink-chain");
    sandbox.plant();

    let outside_dir = sandbox.outer.join("outside-dir");
    fs::create_dir_all(&outside_dir).unwrap();
    fs::write(outside_dir.join("secret.txt"), OUTSIDE).unwrap();

    // Chain: a → b → outside file.
    symlink(sandbox.root().join("b"), sandbox.root().join("a")).unwrap();
    symlink(
        sandbox.outside_path("outside.txt"),
        sandbox.root().join("b"),
    )
    .unwrap();

    for arg in ["a", "b"] {
        let error = sandbox.ws.read(arg, 4096).expect_err("chain must refuse");
        assert!(
            matches!(error, HarnessError::SymlinkRefused { .. }),
            "{error:?}"
        );
    }

    // Symlink in a parent component: dir_link/… never resolves outside.
    symlink(&outside_dir, sandbox.root().join("dir")).unwrap();
    let error = sandbox
        .ws
        .read("dir/secret.txt", 4096)
        .expect_err("parent-component link must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    let error = sandbox
        .ws
        .write("dir/planted.txt", b"escape")
        .expect_err("write through a parent-component link must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    assert!(
        !outside_dir.join("planted.txt").exists(),
        "nothing planted outside"
    );

    // Traversal starting from a name, then escaping above the root.
    let error = sandbox
        .ws
        .read("sub/../../../outside.txt", 4096)
        .expect_err("must refuse");
    assert!(
        matches!(error, HarnessError::PathOutsideRoot { .. }),
        "{error:?}"
    );
}
