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
        "./ok.txt",
        "sub/../.",
        "ok.txt/",
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
