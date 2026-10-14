use std::fs;

use saya_harness::coding::integration::{
    DEFAULT_DELTA_DEADLINE, DeltaError, DeltaKind, candidate_source_digest, compute_delta,
};

use super::delta::active;

#[tokio::test]
async fn output_root_contents_are_not_a_change() {
    let a = active("w07-output-baseline", "checkout-w07-output-baseline").await;
    let before = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    fs::create_dir_all(a.root.join(".saya/home")).unwrap();
    fs::create_dir_all(a.root.join(".saya/tmp")).unwrap();
    fs::write(a.root.join(".saya/home/h.txt"), b"home\n").unwrap();
    fs::write(a.root.join(".saya/tmp/t.txt"), b"tmp\n").unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert!(delta.entries().is_empty());
    let after = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn output_root_is_only_excluded_at_the_root() {
    let a = active("w07-output-nested", "checkout-w07-output-nested").await;
    fs::create_dir_all(a.root.join("sub/.saya")).unwrap();
    fs::write(a.root.join("sub/.saya/x.txt"), b"x\n").unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(delta.entries().len(), 1);
    assert_eq!(delta.entries()[0].path(), "sub/.saya/x.txt");
    assert_eq!(delta.entries()[0].kind(), DeltaKind::Added);
}

#[tokio::test]
async fn a_regular_file_named_dot_saya_is_a_normal_change() {
    let a = active("w07-output-file", "checkout-w07-output-file").await;
    if a.root.join(".saya").is_dir() {
        fs::remove_dir_all(a.root.join(".saya")).unwrap();
    }
    fs::write(a.root.join(".saya"), b"plain file\n").unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(delta.entries().len(), 1);
    assert_eq!(delta.entries()[0].path(), ".saya");
    assert_eq!(delta.entries()[0].kind(), DeltaKind::Added);
}

#[cfg(unix)]
#[tokio::test]
async fn output_root_symlink_and_nested_git_are_refused() {
    let a = active("w07-output-hostile", "checkout-w07-output-hostile").await;
    fs::create_dir_all(a.root.join(".saya")).unwrap();
    std::os::unix::fs::symlink("/etc/hosts", a.root.join(".saya/link")).unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(delta.err().unwrap(), DeltaError::Symlink));
    fs::remove_file(a.root.join(".saya/link")).unwrap();
    fs::create_dir_all(a.root.join(".saya/home/.git")).unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(delta.err().unwrap(), DeltaError::NestedRepository));
}

#[tokio::test]
async fn output_root_bytes_are_bounded() {
    let a = active("w07-output-bytes", "checkout-w07-output-bytes").await;
    fs::create_dir_all(a.root.join(".saya")).unwrap();
    fs::File::create(a.root.join(".saya/big.bin"))
        .unwrap()
        .set_len(65 * 1024 * 1024)
        .unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(
        delta.err().unwrap(),
        DeltaError::OutputRootTooLarge
    ));
}

#[tokio::test]
async fn output_root_entries_are_bounded() {
    let a = active("w07-output-entries", "checkout-w07-output-entries").await;
    let dir = a.root.join(".saya/many");
    fs::create_dir_all(&dir).unwrap();
    for i in 0..4097 {
        fs::write(dir.join(format!("f{i:04}")), b"x").unwrap();
    }
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(
        delta.err().unwrap(),
        DeltaError::OutputRootTooLarge
    ));
}

#[tokio::test]
async fn output_root_honours_cancellation() {
    let a = active("w07-output-cancel", "checkout-w07-output-cancel").await;
    fs::create_dir_all(a.root.join(".saya/home")).unwrap();
    fs::write(a.root.join(".saya/home/h.txt"), b"home\n").unwrap();
    a.token.cancel();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(delta.err().unwrap(), DeltaError::Cancelled));
}
