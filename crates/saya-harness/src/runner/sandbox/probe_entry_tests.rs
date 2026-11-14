use super::OwnedProbeDirectory;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn owned_probe_cleanup_preserves_existing_nonempty_and_recreated_entries() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-probe-owned-{}-{stamp}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let existing = root.join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("sentinel"), b"preexisting synthetic source").unwrap();
    assert!(OwnedProbeDirectory::create(&root, "existing").is_err());
    assert_eq!(
        fs::read(existing.join("sentinel")).unwrap(),
        b"preexisting synthetic source"
    );

    // A child that never started leaves only the newly owned namespace.
    let absent = OwnedProbeDirectory::create(&root, "no-child").unwrap();
    drop(absent);
    assert!(!root.join("no-child").exists());

    let mut replaced = OwnedProbeDirectory::create(&root, "replaced").unwrap();
    fs::rename(root.join("replaced"), root.join("renamed-owned")).unwrap();
    fs::create_dir(root.join("replaced")).unwrap();
    let cleaned = replaced.cleanup();
    let replacement_retained = root.join("replaced").is_dir();
    let original_retained = root.join("renamed-owned").is_dir();
    assert!(
        !cleaned && replacement_retained && original_retained,
        "cleanup must retain a replacement and the original renamed namespace: cleaned={cleaned}, replacement_retained={replacement_retained}, original_retained={original_retained}; evidence {}",
        root.display()
    );

    let mut nonempty = OwnedProbeDirectory::create(&root, "nonempty").unwrap();
    fs::create_dir(nonempty.target()).unwrap();
    assert!(nonempty.record_created_target());
    fs::write(
        nonempty.target().join("foreign-data"),
        b"retained synthetic data",
    )
    .unwrap();
    let retained = nonempty.target().join("foreign-data");
    assert!(!nonempty.cleanup());
    drop(nonempty);
    assert_eq!(fs::read(retained).unwrap(), b"retained synthetic data");

    let mut unknown = OwnedProbeDirectory::create(&root, "unknown-target").unwrap();
    fs::create_dir(unknown.target()).unwrap();
    let unknown_target = unknown.target().to_owned();
    assert!(!unknown.cleanup());
    drop(unknown);
    assert!(
        unknown_target.is_dir(),
        "an unproven target must be retained"
    );

    let mut replaced_child = OwnedProbeDirectory::create(&root, "replaced-child").unwrap();
    fs::create_dir(replaced_child.target()).unwrap();
    assert!(replaced_child.record_created_target());
    fs::rename(replaced_child.target(), root.join("renamed-child")).unwrap();
    fs::create_dir(replaced_child.target()).unwrap();
    let replacement_target = replaced_child.target().to_owned();
    assert!(!replaced_child.cleanup());
    drop(replaced_child);
    assert!(replacement_target.is_dir());
    assert!(root.join("renamed-child").is_dir());

    let mut completed = OwnedProbeDirectory::create(&root, "completed").unwrap();
    fs::create_dir(completed.target()).unwrap();
    assert!(completed.record_created_target());
    assert!(completed.cleanup());
    fs::create_dir(root.join("completed")).unwrap();
    drop(completed);
    assert!(
        root.join("completed").is_dir(),
        "a completed guard must not delete a subsequently recreated entry"
    );
    fs::remove_dir_all(root).unwrap();
}
