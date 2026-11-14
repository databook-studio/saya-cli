use std::{
    fs::{self, File},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use saya_agent::CancellationToken;

use super::super::{NpmCheckSource, NpmRuntimeError, npm_snapshot};

fn source_tree(
    label: &str,
    files: usize,
    bytes_per_file: u64,
) -> (PathBuf, crate::workspace::Workspace) {
    let root = super::private_root(label);
    let project_root = root.join("project");
    fs::create_dir(&project_root).unwrap();
    for index in 0..files {
        let path = project_root.join(format!("part-{index:04}.bin"));
        File::create(path).unwrap().set_len(bytes_per_file).unwrap();
    }
    let workspace = crate::workspace::Workspace::open(&project_root).unwrap();
    (root, workspace)
}

fn source_digest(workspace: &crate::workspace::Workspace) -> String {
    crate::coding::integration::operator_project_source_digest(
        workspace.root(),
        &crate::coding::git::IgnoredPaths::none(),
        &CancellationToken::new(),
        Duration::from_secs(30),
    )
    .unwrap()
    .as_str()
    .to_owned()
}

#[test]
fn project_snapshot_excludes_root_git_metadata_and_managed_mode_remains_strict() {
    let (root, workspace) = source_tree("npm-snapshot-root-git", 1, 1);
    let digest = source_digest(&workspace);
    fs::create_dir(workspace.root().join(".git")).unwrap();
    fs::write(workspace.root().join(".git/config"), b"repository metadata").unwrap();
    let destination = root.join("operator-snapshot");
    let copied = npm_snapshot::copy_source(
        &workspace,
        NpmCheckSource::OperatorProject,
        &destination,
        &digest,
        &CancellationToken::new(),
        Duration::from_secs(30),
    )
    .expect("operator copy accepts root Git metadata");
    assert!(!copied.path.join(".git").exists());
    assert!(copied.path.join("part-0000.bin").is_file());

    let managed = npm_snapshot::copy_source(
        &workspace,
        NpmCheckSource::ManagedCandidate,
        &root.join("managed-snapshot"),
        &digest,
        &CancellationToken::new(),
        Duration::from_secs(30),
    );
    assert!(matches!(managed, Err(NpmRuntimeError::InvalidProject)));
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bounded_source_copy_refuses_more_than_64_mib() {
    let eight_mib = 8 * 1024 * 1024;
    let (root, workspace) = source_tree("npm-snapshot-over-limit", 8, eight_mib);
    File::create(workspace.root().join("part-9999.bin"))
        .unwrap()
        .set_len(1)
        .unwrap();
    let digest = source_digest(&workspace);
    let result = npm_snapshot::copy_source(
        &workspace,
        NpmCheckSource::OperatorProject,
        &root.join("snapshot"),
        &digest,
        &CancellationToken::new(),
        Duration::from_secs(30),
    );

    let refused = matches!(result, Err(NpmRuntimeError::InvalidProject));
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
    assert!(refused, "the copy budget is 64 MiB inclusive");
}

#[test]
fn source_copy_preserves_deadline_as_a_preparation_timeout() {
    let (root, workspace) = source_tree("npm-snapshot-deadline", 1, 1);
    let digest = source_digest(&workspace);
    let result = npm_snapshot::copy_source(
        &workspace,
        NpmCheckSource::OperatorProject,
        &root.join("snapshot"),
        &digest,
        &CancellationToken::new(),
        Duration::ZERO,
    );

    let timed_out = matches!(result, Err(NpmRuntimeError::PreparationTimedOut));
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
    assert!(timed_out);
}

#[test]
fn source_copy_preserves_cancellation_during_fingerprinting() {
    let (root, workspace) = source_tree("npm-snapshot-cancel", 1_000, 64 * 1024);
    let digest = source_digest(&workspace);
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_watching = Arc::clone(&stop);
    let first_copy = root.join("snapshot/part-0000.bin");
    let canceller = thread::spawn(move || {
        while !stop_watching.load(Ordering::Relaxed) {
            if first_copy.exists() {
                cancel.cancel();
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
    });
    let result = npm_snapshot::copy_source(
        &workspace,
        NpmCheckSource::OperatorProject,
        &root.join("snapshot"),
        &digest,
        &cancellation,
        Duration::from_secs(30),
    );
    stop.store(true, Ordering::Relaxed);
    canceller.join().unwrap();

    let cancelled = matches!(result, Err(NpmRuntimeError::Cancelled));
    let copied_files = fs::read_dir(root.join("snapshot"))
        .map(|entries| entries.count())
        .unwrap_or(0);
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
    assert!(cancelled);
    assert!(
        copied_files < 1_000,
        "cancellation stops the bounded copy walk"
    );
}
