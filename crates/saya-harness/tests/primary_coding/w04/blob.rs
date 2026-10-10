use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::{GitRepository, GitService},
    snapshot::{
        CapturedFile, SnapshotContentSource, SnapshotError, SnapshotFileEntry,
        SnapshotOmissionPolicy, SnapshotSelection, capture_head_blob,
    },
};
use saya_types::SnapshotId;

use super::super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};
use super::fixtures::digest;

/// Builds one real Git repository, commits `contents` as `tracked.txt`,
/// plans with the given selection through the public W03 producer, and
/// hands over the service, repository, and pinned entry.
async fn plan_entry(
    label: &str,
    contents: &[u8],
    selection: SnapshotSelection,
) -> (TempRoot, GitService, GitRepository, SnapshotFileEntry) {
    let temp = TempRoot::new(label);
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let (service, opened, entry) =
        plan_committed_entry(&temp, &git, &root, label, contents, selection).await;
    (temp, service, opened, entry)
}

/// Plans `tracked.txt` in an already-created repository at `root`: writes
/// the contents, commits them for a Head selection, and plans through the
/// public W03 producer.
pub(super) async fn plan_committed_entry(
    temp: &TempRoot,
    git: &Path,
    root: &Path,
    label: &str,
    contents: &[u8],
    selection: SnapshotSelection,
) -> (GitService, GitRepository, SnapshotFileEntry) {
    fs::write(root.join("tracked.txt"), contents).unwrap();
    if selection == SnapshotSelection::Head {
        run_git(
            git,
            temp.path(),
            git_args(root, &["add", "--", "tracked.txt"]),
        );
        run_git(
            git,
            temp.path(),
            git_args(root, &["commit", "--quiet", "-m", "tracked"]),
        );
    }
    let service = GitService::new(git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &opened,
            SnapshotId::parse(label).unwrap(),
            selection,
            policy,
            &cancellation,
        )
        .await
        .unwrap();
    let entry = manifest
        .entries()
        .iter()
        .find(|entry| entry.path().as_str() == "tracked.txt")
        .expect("the fixture file is selected")
        .clone();
    (service, opened, entry)
}

/// The loose object file Git stores the planned blob under.
pub(super) fn blob_object_path(root: &Path, entry: &SnapshotFileEntry) -> PathBuf {
    let object_id = match entry.source() {
        SnapshotContentSource::GitBlob { object_id } => object_id,
        _ => panic!("the planned entry is a Git blob"),
    };
    let hex = object_id.as_str();
    root.join(".git")
        .join("objects")
        .join(&hex[..2])
        .join(&hex[2..])
}

pub(super) async fn capture(
    service: &GitService,
    repository: &GitRepository,
    entry: &SnapshotFileEntry,
    remaining: u64,
    sink: &mut dyn Write,
) -> Result<CapturedFile, SnapshotError> {
    let cancellation = CancellationToken::new();
    capture_head_blob(service, repository, entry, remaining, sink, &cancellation).await
}

#[tokio::test]
async fn head_blob_capture_matches_committed_bytes_and_independent_digest() {
    let contents: Vec<u8> = (0..=u8::MAX).cycle().take(150_000).collect();
    let (_temp, service, opened, entry) =
        plan_entry("w04-blob-bytes", &contents, SnapshotSelection::Head).await;
    // A dirty working-tree overlay left after planning: capture must take
    // the committed blob bytes and ignore the overlay entirely.
    fs::write(opened.root().join("tracked.txt"), b"dirty overlay bytes\n").unwrap();
    assert!(matches!(
        entry.source(),
        SnapshotContentSource::GitBlob { .. }
    ));
    let mut sink = Vec::new();
    let captured = capture(&service, &opened, &entry, u64::MAX, &mut sink)
        .await
        .unwrap();
    assert_eq!(sink, contents);
    assert_eq!(captured.bytes(), contents.len() as u64);
    assert_eq!(captured.sha256().as_str(), digest(&contents).as_str());
}

/// The capture succeeds at the exact budget; then, with the loose object
/// removed so Git could not read the blob at all, a one-byte-short budget
/// still refuses with `TooLarge`. A capture that invoked Git first would
/// fail with a missing-object error instead, so the size check provably
/// runs before the Git call.
#[tokio::test]
async fn aggregate_budget_smaller_than_blob_refuses_before_git_call() {
    let contents = b"budget probe bytes\n";
    let (_temp, service, opened, entry) =
        plan_entry("w04-blob-budget", contents, SnapshotSelection::Head).await;
    let mut sink = Vec::new();
    let fits = capture(&service, &opened, &entry, contents.len() as u64, &mut sink)
        .await
        .unwrap();
    assert_eq!(sink, contents);
    assert_eq!(fits.bytes(), contents.len() as u64);
    // The loose object is the only copy of the blob in this fresh
    // repository: after removal Git cannot serve the bytes at all.
    fs::remove_file(blob_object_path(opened.root(), &entry)).unwrap();
    let mut sink = Vec::new();
    let refused = capture(
        &service,
        &opened,
        &entry,
        contents.len() as u64 - 1,
        &mut sink,
    )
    .await
    .expect_err("a budget one byte short refuses the capture");
    assert!(matches!(refused, SnapshotError::TooLarge));
    assert!(sink.is_empty());
}

#[tokio::test]
async fn working_tree_entry_is_refused_by_the_blob_capture() {
    let contents = b"working tree bytes\n";
    let (_temp, service, opened, entry) = plan_entry(
        "w04-blob-worktree",
        contents,
        SnapshotSelection::CurrentWorkingContent,
    )
    .await;
    assert!(matches!(entry.source(), SnapshotContentSource::WorkingTree));
    let mut sink = Vec::new();
    let refused = capture(&service, &opened, &entry, u64::MAX, &mut sink)
        .await
        .expect_err("a working-tree entry is not a blob capture");
    assert!(matches!(refused, SnapshotError::SourceNotGitBlob));
    assert!(sink.is_empty());
}

/// The loose object behind the planned blob is corrupted after planning:
/// the capture refuses with a typed error instead of panicking or serving
/// corrupted bytes, and the sink stays empty.
#[tokio::test]
async fn corrupted_loose_object_refuses() {
    let contents = b"committed blob bytes\n";
    let (_temp, service, opened, entry) =
        plan_entry("w04-blob-corrupt", contents, SnapshotSelection::Head).await;
    let object_path = blob_object_path(opened.root(), &entry);
    fs::remove_file(&object_path).unwrap();
    fs::write(&object_path, b"corrupted object bytes").unwrap();
    let mut sink = Vec::new();
    let refused = capture(&service, &opened, &entry, u64::MAX, &mut sink).await;
    assert!(refused.is_err(), "corrupt object refuses");
    assert!(sink.is_empty());
}

struct CancelOnFirstWrite<'a> {
    token: &'a CancellationToken,
    written: Vec<u8>,
}

impl Write for CancelOnFirstWrite<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.token.cancel();
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn cancellation_between_chunks_reports_cancelled() {
    let contents: Vec<u8> = (0..=u8::MAX).cycle().take(3 * 64 * 1024).collect();
    let (_temp, service, opened, entry) =
        plan_entry("w04-blob-cancel", &contents, SnapshotSelection::Head).await;
    let cancellation = CancellationToken::new();
    let mut sink = CancelOnFirstWrite {
        token: &cancellation,
        written: Vec::new(),
    };
    let stopped = capture_head_blob(
        &service,
        &opened,
        &entry,
        u64::MAX,
        &mut sink,
        &cancellation,
    )
    .await
    .expect_err("a cancelled capture stops with Cancelled");
    assert!(matches!(stopped, SnapshotError::Cancelled));
    assert!(!sink.written.is_empty());
    assert!(sink.written.len() <= 64 * 1024);
    assert!(sink.written.len() < contents.len());
    assert_eq!(&contents[..sink.written.len()], &sink.written[..]);
}

/// A sink whose first write fails with caller-controlled text.
struct LeakySink;

impl Write for LeakySink {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("secret /tmp/leak bytes"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn sink_error_is_redacted_in_display_and_debug() {
    let contents = b"payload bytes\n";
    let (_temp, service, opened, entry) =
        plan_entry("w04-blob-sink", contents, SnapshotSelection::Head).await;
    let mut sink = LeakySink;
    let refused = capture(&service, &opened, &entry, u64::MAX, &mut sink)
        .await
        .expect_err("a failing sink refuses the capture");
    assert!(matches!(refused, SnapshotError::CaptureIo { .. }));
    for rendered in [format!("{refused}"), format!("{refused:?}")] {
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("/tmp"));
    }
}
