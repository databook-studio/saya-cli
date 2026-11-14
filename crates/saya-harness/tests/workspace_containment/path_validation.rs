use super::*;

#[test]
fn reads_and_writes_round_trip_inside_the_root() {
    let sandbox = Sandbox::new("roundtrip");
    sandbox.plant();

    let read = sandbox.ws.read("ok.txt", 4096).unwrap();
    assert_eq!(read.bytes, INSIDE);
    assert_eq!(read.size, INSIDE.len() as u64);
    assert!(!read.truncated);

    let nested = sandbox.ws.read("sub/nested.txt", 4096).unwrap();
    assert_eq!(nested.bytes, INSIDE);

    sandbox.ws.write("deep/dir/file.txt", b"created").unwrap();
    assert_eq!(
        sandbox.ws.read("deep/dir/file.txt", 4096).unwrap().bytes,
        b"created"
    );
}

#[test]
fn root_is_resolved_once_and_canonical() {
    let sandbox = Sandbox::new("root");
    assert_eq!(sandbox.root(), fs::canonicalize(sandbox.root()).unwrap());
}

#[test]
fn argument_validation_rejects_malformed_shapes() {
    let sandbox = Sandbox::new("validation");
    sandbox.plant();

    let cases = [
        "",
        "a\0b",
        ".",
        "./",
        "sub//nested.txt",
        "C:\\outside.txt",
        "C:/outside.txt",
        "\\\\srv\\share\\file",
    ];
    for arg in cases {
        let read = sandbox.ws.read(arg, 4096).expect_err("must reject");
        assert!(
            matches!(read, HarnessError::InvalidPath { .. }),
            "read({arg:?}) → {read:?}"
        );
        let write = sandbox.ws.write(arg, b"x").expect_err("must reject");
        assert!(
            matches!(write, HarnessError::InvalidPath { .. }),
            "write({arg:?}) → {write:?}"
        );
    }
}

#[test]
fn equivalent_relative_spellings_resolve_inside_the_root() {
    let sandbox = Sandbox::new("aliases");
    sandbox.plant();

    for arg in [
        "./ok.txt",
        "ok.txt/",
        "sub/./nested.txt",
        "./sub/nested.txt",
    ] {
        let read = sandbox.ws.read(arg, 4096).expect("alias must resolve");
        assert_eq!(read.bytes, INSIDE, "read({arg:?})");
    }
    let listed = sandbox
        .ws
        .list("sub/", MAX_LIST_ENTRIES)
        .expect("a trailing slash must list the directory");
    assert_eq!(listed.entries.len(), 1);
    assert_eq!(listed.entries[0].name, "nested.txt");
    let from_dot = sandbox
        .ws
        .list("./sub", MAX_LIST_ENTRIES)
        .expect("./sub must list the same directory");
    assert_eq!(from_dot.entries.len(), 1);
    assert_eq!(from_dot.entries[0].name, "nested.txt");
}

/// An absolute path that resolves under the root is the same file as its
/// relative spelling: the root is stripped and every existing containment
/// check runs on the rest. An absolute path outside the root is refused, and
/// the refusal names the root.
#[test]
fn an_absolute_path_under_the_root_resolves_and_one_outside_names_the_root() {
    let sandbox = Sandbox::new("absolute");
    sandbox.plant();
    let root = sandbox.root().to_str().expect("the root is UTF-8");

    let inside = sandbox.root().join("sub/nested.txt");
    let read = sandbox
        .ws
        .read(inside.to_str().expect("the path is UTF-8"), 4096)
        .expect("an absolute path under the root must resolve");
    assert_eq!(read.bytes, INSIDE);

    let created = sandbox.root().join("deep/abs.txt");
    sandbox
        .ws
        .write(created.to_str().expect("the path is UTF-8"), b"absolute")
        .expect("an absolute write under the root must resolve");
    assert_eq!(
        sandbox.ws.read("deep/abs.txt", 4096).unwrap().bytes,
        b"absolute"
    );

    let listed = sandbox
        .ws
        .list(root, MAX_LIST_ENTRIES)
        .expect("the absolute root must list");
    assert!(!listed.entries.is_empty());

    // `..` in an accepted absolute path still cannot rise above the root.
    let escape = sandbox.root().join("../outside.txt");
    let error = sandbox
        .ws
        .read(escape.to_str().expect("the path is UTF-8"), 4096)
        .expect_err("an absolute path rising above the root must refuse");
    assert!(
        matches!(error, HarnessError::PathOutsideRoot { .. }),
        "{error:?}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        // A symlink under the root is refused by the same component walk.
        symlink(
            sandbox.outside_path("outside.txt"),
            sandbox.root().join("abs-link"),
        )
        .unwrap();
        let link = sandbox.root().join("abs-link");
        let error = sandbox
            .ws
            .read(link.to_str().expect("the path is UTF-8"), 4096)
            .expect_err("an absolute path through a symlink must refuse");
        assert!(
            matches!(error, HarnessError::SymlinkRefused { .. }),
            "{error:?}"
        );

        // An absolute path outside the root is refused as before, naming the root.
        let error = sandbox
            .ws
            .read("/etc/passwd", 4096)
            .expect_err("an absolute path outside the root must refuse");
        let text = error.to_string();
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "{error:?}"
        );
        assert!(text.contains("escapes the run workspace"), "{text}");
        assert!(text.contains(root), "the refusal names the root: {text}");
    }
}

#[test]
fn absolute_paths_and_dotdot_above_root_refuse() {
    let sandbox = Sandbox::new("escape");
    sandbox.plant();

    for arg in ["../outside.txt", "..", "sub/../../outside.txt"] {
        let error = sandbox.ws.read(arg, 4096).expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "read({arg:?}) → {error:?}"
        );
        let error = sandbox.ws.write(arg, b"escape").expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "write({arg:?}) → {error:?}"
        );
    }
    #[cfg(unix)]
    {
        let arg = "/etc/passwd";
        let error = sandbox.ws.read(arg, 4096).expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "read({arg:?}) → {error:?}"
        );
        let error = sandbox.ws.write(arg, b"escape").expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "write({arg:?}) → {error:?}"
        );
    }
    assert_eq!(
        fs::read(sandbox.outside_path("outside.txt")).unwrap(),
        OUTSIDE
    );
}
