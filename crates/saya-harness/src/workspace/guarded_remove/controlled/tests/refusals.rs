use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

// These exercise the real controlled owner with actual targets and retained
// bytes; no other adapter's denial is used as private/type/digest proof.
#[test]
fn controlled_refusals_preserve_existing_files_and_parent_directories() {
    for (path, bytes, expected) in [
        (
            ".env",
            b"ordinary synthetic data\n".as_slice(),
            RemovalRefusal::Private,
        ),
        (
            ".codex/context.md",
            b"ordinary synthetic data\n".as_slice(),
            RemovalRefusal::Private,
        ),
        (
            "ordinary.rs",
            b"password=synthetic-private-control-P31\n".as_slice(),
            RemovalRefusal::Private,
        ),
        (
            "export.sqlite",
            b"ordinary synthetic data\n".as_slice(),
            RemovalRefusal::Private,
        ),
    ] {
        let temp = TempRoot::new();
        let workspace = Workspace::open(temp.path()).unwrap();
        fs::create_dir_all(temp.path().join(path).parent().unwrap()).unwrap();
        fs::write(temp.path().join(path), bytes).unwrap();
        let digest = workspace.read(path, 0).unwrap().digest;
        assert!(
            matches!(workspace.prepare_guarded_remove(path,&digest,CancellationToken::new(),
            Instant::now()+Duration::from_secs(5)),Err(ControlledRemoveError::NoEffect(reason)) if reason==expected)
        );
        assert_eq!(fs::read(temp.path().join(path)).unwrap(), bytes);
    }
    let (temp, workspace, digest) = target();
    fs::create_dir(temp.path().join("directory")).unwrap();
    symlink(temp.path().join("victim.rs"), temp.path().join("link.rs")).unwrap();
    for path in ["missing.rs", "directory", "link.rs"] {
        assert!(matches!(
            workspace.prepare_guarded_remove(
                path,
                &digest,
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(5)
            ),
            Err(ControlledRemoveError::NoEffect(RemovalRefusal::Target))
        ));
    }
    for (path, expected) in [
        ("../victim.rs", digest.as_str()),
        ("victim.rs", "invalid"),
        ("victim.rs", &"A".repeat(64)),
    ] {
        assert!(matches!(
            workspace.prepare_guarded_remove(
                path,
                expected,
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(5)
            ),
            Err(ControlledRemoveError::NoEffect(RemovalRefusal::Invalid))
        ));
    }
    fs::write(temp.path().join("victim.rs"), b"changed source!\n").unwrap();
    assert!(matches!(
        workspace.prepare_guarded_remove(
            "victim.rs",
            &digest,
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5)
        ),
        Err(ControlledRemoveError::NoEffect(RemovalRefusal::Changed))
    ));
    assert_eq!(
        fs::read(temp.path().join("victim.rs")).unwrap(),
        b"changed source!\n"
    );
    assert!(temp.path().join("directory").is_dir());
    assert!(temp.path().join("link.rs").is_symlink());
}

#[test]
fn controlled_removal_enforces_actual_size_bound_and_keeps_empty_parents() {
    let temp = TempRoot::new();
    let workspace = Workspace::open(temp.path()).unwrap();
    let cap = crate::workspace::MAX_IO_BYTES;
    fs::write(temp.path().join("large.rs"), vec![b'x'; cap + 1]).unwrap();
    assert!(matches!(
        workspace.prepare_guarded_remove(
            "large.rs",
            &"a".repeat(64),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5)
        ),
        Err(ControlledRemoveError::NoEffect(RemovalRefusal::TooLarge))
    ));
    assert_eq!(
        fs::metadata(temp.path().join("large.rs")).unwrap().len(),
        (cap + 1) as u64
    );
    fs::create_dir(temp.path().join("source")).unwrap();
    fs::write(temp.path().join("source/empty.rs"), b"").unwrap();
    let digest = workspace.read("source/empty.rs", 0).unwrap().digest;
    workspace
        .prepare_guarded_remove(
            "source/empty.rs",
            &digest,
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap()
        .commit()
        .unwrap();
    assert!(!temp.path().join("source/empty.rs").exists());
    assert!(temp.path().join("source").is_dir());
}

#[test]
fn opaque_unlink_failure_is_unknown_and_preserves_the_target() {
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skip: root bypasses directory write permission");
        return;
    }
    let (temp, workspace, digest) = target();
    let prepared = workspace
        .prepare_guarded_remove(
            "victim.rs",
            &digest,
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let result = prepared.commit();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(result, Err(ControlledRemoveError::Unknown)));
    assert_eq!(
        fs::read(temp.path().join("victim.rs")).unwrap(),
        b"ordinary source\n"
    );
}
