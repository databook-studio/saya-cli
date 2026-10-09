use super::*;

// ---------------------------------------------------------------------------
// 8. nothing_about_trust_is_persisted
// ---------------------------------------------------------------------------

/// Trust is session-only: binding the trusted folder writes no dotfile, no
/// marker, no config key anywhere. The session record carries only the root
/// pin — today's shape — and re-trust is per-process.
#[test]
fn nothing_about_trust_is_persisted() {
    let cwd = temp_dir("trust-no-store");
    let before: Vec<String> = fs::read_dir(&cwd)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let resolved = resolve_trusted_dir(&cwd).expect("the trusted dir resolves");
    let runtime = session_runtime();
    let state = temp_dir("trust-no-store-state");
    let universe = SessionUniverse::compose(&runtime, Some(&resolved), None, true, &cwd, &state)
        .expect("trust binds");
    assert!(universe.root().is_some(), "trust binds");
    let after: Vec<String> = fs::read_dir(&cwd)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        before, after,
        "trust writes no dotfile, no marker into the trusted folder"
    );
    assert!(
        !cwd.join(".saya-trust").exists() && !cwd.join(".saya").exists(),
        "no trust store appears in the folder"
    );
    let _ = (fs::remove_dir_all(&cwd), fs::remove_dir_all(&state));
}
