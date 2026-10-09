use super::*;

#[test]
fn git_names_are_refused_as_hygiene_only() {
    let sandbox = Sandbox::new("git-hygiene");
    sandbox.plant();
    fs::create_dir_all(sandbox.root().join(".git")).unwrap();
    fs::write(sandbox.root().join(".git/config"), INSIDE).unwrap();

    for arg in [".git", ".git/config", ".GIT/config", "sub/.git/config"] {
        let error = sandbox.ws.read(arg, 4096).expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::DeniedName { .. }),
            "{arg:?} → {error:?}"
        );
        let error = sandbox.ws.write(arg, b"nope").expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::DeniedName { .. }),
            "{arg:?} → {error:?}"
        );
    }
    // The manifest walk skips hygiene content silently.
    let manifest = saya_harness::workspace::manifest::build(&sandbox.ws, 100, 1 << 20).unwrap();
    assert!(manifest.iter().all(|entry| !entry.path.contains(".git")));
}

#[cfg(target_os = "macos")]
#[test]
fn nfd_and_case_aliases_resolve_inside_the_root_or_refuse() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("unicode-fold");
    sandbox.plant();

    // An NFD-named file must be reachable through its NFC spelling: the
    // kernel folds the lookup, and canonicalise + prefix keep it contained.
    let nfd = "r\u{e9}sum\u{e9}.txt"; // composed here; the FS stores NFD bytes
    let nfd_bytes: &str = "re\u{301}sume\u{301}.txt";
    sandbox.ws.write(nfd_bytes, INSIDE).unwrap();
    let aliased = sandbox.ws.read(nfd, 4096);
    assert!(
        aliased.as_ref().is_ok_and(|file| file.bytes == INSIDE) || {
            matches!(
                aliased,
                Err(HarnessError::NotFound { .. }) | Err(HarnessError::Io { .. })
            )
        },
        "NFC/NFD aliasing must stay contained: {aliased:?}"
    );

    // Case-insensitive lookup must not turn a case variant into a new escape:
    // a symlink reached through a folded spelling is still refused.
    symlink(
        sandbox.outside_path("outside.txt"),
        sandbox.root().join("Lin\u{413}k"),
    )
    .unwrap();
    for spelling in ["Lin\u{413}k", "li\u{413}k", "LIN\u{413}K"] {
        match sandbox.ws.read(spelling, 4096) {
            Ok(file) => assert_eq!(file.bytes, INSIDE, "folded read must stay inside"),
            Err(error) => {
                assert!(is_rejection(&error), "{spelling} → {error}");
                if matches!(error, HarnessError::SymlinkRefused { .. }) {
                    // The folded lookup landed on the link and was refused.
                }
            }
        }
    }
}

#[test]
#[cfg(all(not(target_os = "macos"), not(windows)))]
fn case_variants_do_not_alias_on_case_sensitive_filesystems() {
    let sandbox = Sandbox::new("case-sensitive");
    sandbox.plant();
    // An absent case variant is absence, not an escape: the typed `NotFound`
    // carries the workspace-relative name, exactly as the old `Io`-kind
    // `NotFound` did.
    assert!(matches!(
        sandbox.ws.read("OK.TXT", 4096),
        Err(HarnessError::NotFound { path }) if path == "OK.TXT"
    ));
}

/// Absence keeps the workspace-relative name in the typed error: no absolute
/// host path reaches the `NotFound` payload.
#[test]
fn a_missing_file_reports_its_workspace_relative_name() {
    let sandbox = Sandbox::new("not-found-path");
    sandbox.plant();
    assert!(matches!(
        sandbox.ws.read("absent.txt", 4096),
        Err(HarnessError::NotFound { path }) if path == "absent.txt"
    ));
    assert!(matches!(
        sandbox.ws.read("sub/absent.txt", 4096),
        Err(HarnessError::NotFound { path }) if path == "sub/absent.txt"
    ));
    assert!(matches!(
        sandbox.ws.read("no/such/dir.txt", 4096),
        Err(HarnessError::NotFound { path }) if path == "no/such/dir.txt"
    ));
}
