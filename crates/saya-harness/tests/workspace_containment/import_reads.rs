use super::*;

#[test]
fn scratch_import_read_has_its_own_32_mib_cap_and_containment() {
    let sandbox = Sandbox::new("scratch-import-read");
    let medium = vec![b'x'; MAX_IO_BYTES + 1];
    fs::write(sandbox.root().join("medium.csv"), &medium).unwrap();
    assert!(matches!(
        sandbox.ws.read("medium.csv", MAX_IO_BYTES as u64),
        Err(HarnessError::BoundsExceeded { .. })
    ));
    let imported = sandbox.ws.read_for_scratch_import("medium.csv").unwrap();
    assert_eq!(imported.bytes, medium);
    assert!(!imported.truncated);

    let too_large = fs::File::create(sandbox.root().join("large.csv")).unwrap();
    too_large.set_len(32 * 1024 * 1024 + 1).unwrap();
    assert!(matches!(
        sandbox.ws.read_for_scratch_import("large.csv"),
        Err(HarnessError::BoundsExceeded { max, .. }) if max == 32 * 1024 * 1024
    ));
    assert!(is_rejection(
        &sandbox
            .ws
            .read_for_scratch_import("../outside.csv")
            .unwrap_err()
    ));
}

#[cfg(unix)]
#[test]
fn scratch_import_read_refuses_an_outside_symlink() {
    let sandbox = Sandbox::new("scratch-import-symlink");
    fs::write(sandbox.outside_path("outside.csv"), OUTSIDE).unwrap();
    std::os::unix::fs::symlink(
        sandbox.outside_path("outside.csv"),
        sandbox.root().join("link.csv"),
    )
    .unwrap();
    assert!(matches!(
        sandbox.ws.read_for_scratch_import("link.csv"),
        Err(HarnessError::SymlinkRefused { .. })
    ));
}
