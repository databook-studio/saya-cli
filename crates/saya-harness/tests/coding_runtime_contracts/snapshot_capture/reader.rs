use std::{
    fs,
    io::Write,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{
        CapturedFile, MAX_SNAPSHOT_FILE_BYTES, SnapshotContentSource, SnapshotError,
        SnapshotFileEntry, SnapshotOmissionPolicy, SnapshotSelection, read_working_file,
    },
};
use saya_harness::workspace::Workspace;
use saya_types::SnapshotId;
use sha2::{Digest, Sha256};

use super::super::support::{TempRoot, git_executable, repository, state_root};

fn capture(
    entry: &SnapshotFileEntry,
    workspace: &Workspace,
    sink: &mut dyn Write,
    remaining: u64,
) -> Result<CapturedFile, SnapshotError> {
    read_working_file(workspace, entry, remaining, sink, &CancellationToken::new())
}

/// Plans the fixture's committed `tracked.txt` with `contents` through the
/// public W03 producer, giving every test the exact entry shape W04d hands over.
async fn plan_working_entry(
    label: &str,
    contents: &[u8],
) -> (TempRoot, PathBuf, SnapshotFileEntry) {
    plan_selection(
        label,
        |root| fs::write(root.join("tracked.txt"), contents).unwrap(),
        SnapshotSelection::CurrentWorkingContent,
    )
    .await
}

/// Builds one real Git repository, lets `prepare` shape the working tree,
/// then plans it and hands over the pinned `tracked.txt` entry.
async fn plan_selection(
    label: &str,
    prepare: impl FnOnce(&Path),
    selection: SnapshotSelection,
) -> (TempRoot, PathBuf, SnapshotFileEntry) {
    let temp = TempRoot::new(label);
    let git = git_executable();
    let root = repository(temp.path(), &git);
    prepare(&root);
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
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
    (temp, root, entry)
}

#[tokio::test]
async fn working_file_capture_hashes_in_one_open() {
    let contents: Vec<u8> = (0..=u8::MAX).cycle().take(150_000).collect();
    let (_temp, root, entry) = plan_working_entry("w04-hash", &contents).await;
    let workspace = Workspace::open(&root).unwrap();
    let mut sink = Vec::new();
    let captured = capture(&entry, &workspace, &mut sink, u64::MAX).unwrap();
    assert_eq!(sink, contents);
    assert_eq!(captured.bytes(), contents.len() as u64);
    let expected: String = Sha256::digest(&contents)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(captured.sha256().as_str(), expected);
    let rendered = format!("{captured:?}");
    assert!(rendered.contains(&expected));
    assert!(!rendered.contains("tracked"));
}

#[tokio::test]
async fn identity_change_between_plan_and_capture_refuses() {
    let (_temp, root, entry) = plan_working_entry("w04-identity", b"same bytes\n").await;
    let workspace = Workspace::open(&root).unwrap();
    let path = root.join("tracked.txt");
    let mut sink = Vec::new();
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"same bytes\n").unwrap();
    let swapped = capture(&entry, &workspace, &mut sink, u64::MAX);
    assert!(matches!(swapped, Err(SnapshotError::SourceIdentityChanged)));
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"different size\n").unwrap();
    let resized = capture(&entry, &workspace, &mut sink, u64::MAX);
    assert!(matches!(resized, Err(SnapshotError::SourceIdentityChanged)));
    assert!(sink.is_empty());
}

#[tokio::test]
async fn deleted_after_plan_refuses_as_source_disappeared() {
    let (_temp, root, entry) = plan_working_entry("w04-deleted", b"to delete\n").await;
    fs::remove_file(root.join("tracked.txt")).unwrap();
    let workspace = Workspace::open(&root).unwrap();
    let mut sink = Vec::new();
    let refused = capture(&entry, &workspace, &mut sink, u64::MAX);
    assert!(matches!(refused, Err(SnapshotError::SourceDisappeared)));
    assert!(sink.is_empty());
}

#[tokio::test]
async fn symlink_swapped_into_selected_path_refuses() {
    let (temp, root, entry) = plan_working_entry("w04-symlink", b"real bytes\n").await;
    let canary = temp.path().join("canary-outside-root");
    fs::write(&canary, b"canary bytes\n").unwrap();
    fs::remove_file(root.join("tracked.txt")).unwrap();
    std::os::unix::fs::symlink(&canary, root.join("tracked.txt")).unwrap();
    let workspace = Workspace::open(&root).unwrap();
    let mut sink = Vec::new();
    let refused = capture(&entry, &workspace, &mut sink, u64::MAX);
    assert!(matches!(refused, Err(SnapshotError::SourceSymlink)));
    assert!(sink.is_empty());
    assert_eq!(fs::read(&canary).unwrap(), b"canary bytes\n");
}

/// A FIFO swapped into the planned path is refused as a special file by the
/// pre-open scan, before any open is attempted, so the refusal cannot block
/// on the FIFO. The separate nonblocking-open guarantee (an open that did run
/// against a special file uses O_NONBLOCK) is source-reviewed only: this test
/// never reaches `open_verified`.
#[tokio::test]
async fn fifo_swap_refuses_at_the_scan_without_blocking() {
    let (_temp, root, entry) = plan_working_entry("w04-fifo", b"before fifo\n").await;
    let fifo = root.join("tracked.txt");
    fs::remove_file(&fifo).unwrap();
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let workspace = Workspace::open(&root).unwrap();
    let mut sink = Vec::new();
    let refused = capture(&entry, &workspace, &mut sink, u64::MAX);
    assert!(matches!(refused, Err(SnapshotError::SourceNotRegularFile)));
    assert!(sink.is_empty());
}

#[tokio::test]
async fn executable_bit_mismatch_refuses() {
    let (_temp, root, entry) = plan_working_entry("w04-exec", b"script bytes\n").await;
    fs::set_permissions(root.join("tracked.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    let workspace = Workspace::open(&root).unwrap();
    let mut sink = Vec::new();
    let refused = capture(&entry, &workspace, &mut sink, u64::MAX);
    assert!(matches!(refused, Err(SnapshotError::SourceIdentityChanged)));
    assert!(sink.is_empty());
}

/// Reader refusal at per-file cap + 1 is not reachable through the public
/// planner (W03 refuses any entry above `MAX_SNAPSHOT_FILE_BYTES` at plan
/// time), so that boundary is covered by W03's planner test. This test proves
/// success at the cap and aggregate refusal one byte below it.
#[tokio::test]
async fn per_file_cap_and_aggregate_cap_refuse_one_byte_over() {
    let (_temp, root, entry) = plan_selection(
        "w04-caps",
        |root| {
            let file = fs::File::create(root.join("tracked.txt")).unwrap();
            file.set_len(MAX_SNAPSHOT_FILE_BYTES).unwrap();
        },
        SnapshotSelection::CurrentWorkingContent,
    )
    .await;
    let workspace = Workspace::open(&root).unwrap();
    let mut sink = Vec::new();
    let fits = capture(&entry, &workspace, &mut sink, MAX_SNAPSHOT_FILE_BYTES).unwrap();
    assert_eq!(fits.bytes(), MAX_SNAPSHOT_FILE_BYTES);
    sink.clear();
    let over = capture(&entry, &workspace, &mut sink, MAX_SNAPSHOT_FILE_BYTES - 1);
    assert!(matches!(over, Err(SnapshotError::TooLarge)));
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
async fn cancellation_between_chunks_stops_and_reports_cancelled() {
    let contents: Vec<u8> = (0..=u8::MAX).cycle().take(3 * 64 * 1024).collect();
    let (_temp, root, entry) = plan_working_entry("w04-cancel", &contents).await;
    let workspace = Workspace::open(&root).unwrap();
    let cancellation = CancellationToken::new();
    let mut sink = CancelOnFirstWrite {
        token: &cancellation,
        written: Vec::new(),
    };
    let stopped = read_working_file(&workspace, &entry, u64::MAX, &mut sink, &cancellation);
    assert!(matches!(stopped, Err(SnapshotError::Cancelled)));
    assert!(!sink.written.is_empty());
    assert!(sink.written.len() <= 64 * 1024);
    assert!(sink.written.len() < contents.len());
    assert_eq!(&contents[..sink.written.len()], &sink.written[..]);
}

#[tokio::test]
async fn git_blob_entry_is_refused_by_the_working_tree_reader() {
    let (_temp, root, entry) = plan_selection(
        "w04-blob",
        |root| fs::write(root.join("tracked.txt"), b"ignored overlay\n").unwrap(),
        SnapshotSelection::Head,
    )
    .await;
    assert!(matches!(
        entry.source(),
        SnapshotContentSource::GitBlob { .. }
    ));
    assert!(entry.source_identity().is_none());
    let workspace = Workspace::open(&root).unwrap();
    let mut sink = Vec::new();
    let refused = capture(&entry, &workspace, &mut sink, u64::MAX);
    assert!(matches!(refused, Err(SnapshotError::SourceNotWorkingTree)));
    assert!(sink.is_empty());
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
async fn sink_write_error_is_redacted_in_display_and_debug() {
    let (_temp, root, entry) = plan_working_entry("w04-sink", b"payload bytes\n").await;
    let workspace = Workspace::open(&root).unwrap();
    let mut sink = LeakySink;
    let refused = capture(&entry, &workspace, &mut sink, u64::MAX)
        .expect_err("a failing sink refuses the capture");
    assert!(matches!(refused, SnapshotError::CaptureIo { .. }));
    for rendered in [format!("{refused}"), format!("{refused:?}")] {
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("/tmp"));
    }
}
