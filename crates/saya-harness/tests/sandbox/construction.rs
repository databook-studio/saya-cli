use super::*;

/// Construction refuses what the platform's sandbox cannot express — per
/// platform, at construction, never silently narrowed.
#[cfg(not(windows))]
#[test]
fn construction_refuses_the_unexpressible() {
    let root = TempRoot::new("construct");
    let canonical = root.canonical();
    assert!(
        matches!(
            RunSandbox::new(Vec::<PathBuf>::new(), Vec::<(String, u16)>::new()),
            Err(SandboxError::NoRoots)
        ),
        "a policy with no roots is refused"
    );
    assert!(
        matches!(
            RunSandbox::new([canonical.clone()], [("127.0.0.1".to_owned(), 0)]),
            Err(SandboxError::NetPortInvalid { .. })
        ),
        "port 0 is not an enforceable egress declaration"
    );
    assert!(
        matches!(
            RunSandbox::new([canonical.clone()], [("example.com".to_owned(), 443)]),
            Err(SandboxError::NetHostNotExpressible { .. })
        ),
        "a named remote host is not expressible by any platform sandbox in this design"
    );
}

/// Test 5 — Windows has no measured profile language. Windows path spelling
/// is therefore rejected before a policy is created, and the runner remains
/// unavailable by construction.
#[cfg(windows)]
#[test]
fn windows_refuses_a_profile_root_and_never_creates_a_runner_policy() {
    let root = TempRoot::new("windows");
    let canonical = root.canonical();
    assert!(
        matches!(
            RunSandbox::new([canonical], Vec::<(String, u16)>::new()),
            Err(SandboxError::RootNotSafeForProfile { .. })
        ),
        "Windows has no supported sandbox profile path; policy construction must fail closed"
    );
}
