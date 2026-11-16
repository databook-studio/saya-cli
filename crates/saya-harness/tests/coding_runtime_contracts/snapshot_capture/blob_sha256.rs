use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

use saya_harness::coding::snapshot::{SnapshotContentSource, SnapshotError, SnapshotSelection};

use super::super::support::{TempRoot, git_args, git_executable, git_home, run_git};
use super::blob::{blob_object_path, capture, plan_committed_entry};
use super::fixtures::digest;

/// Builds a `std::process::Command` for Git with the support fixture
/// environment — the same `std::process::Command` pattern `run_git` uses —
/// so the sha256 init, read probes, and stdin-fed object writes share one
/// environment.
fn fixture_command(executable: &Path, root: &Path, args: &[&str]) -> Command {
    let home = git_home(root.parent().expect("the repository has a parent"));
    let mut command = Command::new(executable);
    command
        .env_clear()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("PATH", executable.parent().expect("Git has a parent"))
        .env(
            "TMPDIR",
            root.parent().expect("the repository has a parent"),
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("empty.gitconfig"))
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .arg("-c")
        .arg(format!(
            "core.hooksPath={}",
            root.parent()
                .expect("the repository has a parent")
                .join("empty-hooks")
                .display()
        ))
        .args(git_args(root, args));
    command
}

/// Runs Git with the support fixture environment and returns the raw
/// output, so init failures and read probes can inspect the exit status.
fn try_run_git(executable: &Path, root: &Path, args: &[&str]) -> std::process::Output {
    fixture_command(executable, root, args)
        .output()
        .expect("Git starts")
}

/// Initializes `root` as a sha256 repository, failing the test with Git's
/// stderr on any init error except Git's unsupported-object-format
/// diagnostics: only stderr (compared case-insensitively) containing
/// "unknown hash algorithm", "unknown object format", or both
/// "unknown option" and "object-format" skips the caller's test (returns
/// false); every other failure, including an error that merely names the
/// fixture path, panics.
fn init_sha256_repository(temp: &TempRoot, git: &Path, root: &Path) -> bool {
    fs::create_dir(root).unwrap();
    let template = format!(
        "--template={}",
        temp.path().join("empty-template").display()
    );
    let init = try_run_git(
        git,
        root,
        &[
            "init",
            "--quiet",
            "--object-format=sha256",
            template.as_str(),
        ],
    );
    if init.status.success() {
        return true;
    }
    let stderr = String::from_utf8_lossy(&init.stderr).into_owned();
    let lowered = stderr.to_lowercase();
    let unsupported = lowered.contains("unknown hash algorithm")
        || lowered.contains("unknown object format")
        || (lowered.contains("unknown option") && lowered.contains("object-format"));
    assert!(
        unsupported,
        "git init failed for an unrelated reason: {stderr}"
    );
    false
}

/// Runs `git hash-object -w --stdin` with `bytes` on stdin and returns the
/// written object's hex id, so a test can locate the loose object file Git
/// stored it under.
fn write_loose_blob(executable: &Path, root: &Path, bytes: &[u8]) -> String {
    let mut child = fixture_command(executable, root, &["hash-object", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Git starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(bytes)
        .expect("the blob bytes reach Git's stdin");
    let output = child.wait_with_output().expect("Git finishes");
    assert!(
        output.status.success(),
        "git hash-object -w --stdin failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let object_id = String::from_utf8(output.stdout).expect("the object id is UTF-8");
    let trimmed = object_id.trim();
    assert_eq!(trimmed.len(), 64, "the sha256 object id has 64 hex digits");
    trimmed.to_owned()
}

/// A SHA-256 object-format repository exercises the blob-vs-object-id
/// verification path end to end: the capture serves the committed bytes,
/// the digest matches an independent SHA-256, and the capture succeeds
/// without a `BlobDigestMismatch`. Skipped when the installed Git reports
/// the sha256 object format as unsupported.
#[tokio::test]
async fn sha256_repository_blob_capture_matches_committed_bytes() {
    let contents = b"sha256 blob bytes\n";
    let temp = TempRoot::new("w04-blob-sha256");
    let git = git_executable();
    let root = temp.path().join("sha256 repository");
    if !init_sha256_repository(&temp, &git, &root) {
        return;
    }
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["config", "user.name", "W04 fixture"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["config", "user.email", "w04@example.invalid"]),
    );
    let (service, opened, entry) = plan_committed_entry(
        &temp,
        &git,
        &root,
        "w04-blob-sha256",
        contents,
        SnapshotSelection::Head,
    )
    .await;
    // The planned object id is a full 64-hex-digit sha256 id, so the
    // repository really uses the sha256 format and the verification path
    // runs instead of being skipped for sha1.
    let object_id = match entry.source() {
        SnapshotContentSource::GitBlob { object_id } => object_id,
        _ => panic!("the planned entry is a Git blob"),
    };
    assert_eq!(object_id.as_str().len(), 64);
    let mut sink = Vec::new();
    let captured = capture(&service, &opened, &entry, u64::MAX, &mut sink)
        .await
        .unwrap();
    assert_eq!(sink, contents);
    assert_eq!(captured.bytes(), contents.len() as u64);
    assert_eq!(captured.sha256().as_str(), digest(contents).as_str());
}

/// Git does not verify a loose object's digest when it reads one: the
/// committed blob's loose file is overwritten in place with the loose file
/// of a same-length, different-byte blob, after which `git cat-file blob`
/// serves the wrong bytes with no error. The public capture must still
/// refuse them with `BlobDigestMismatch`. Skipped when the installed Git
/// reports the sha256 object format as unsupported.
#[tokio::test]
async fn sha256_repository_wrong_bytes_refuse_with_blob_digest_mismatch() {
    let contents = b"hello";
    let temp = TempRoot::new("w04-blob-sha256-wrong");
    let git = git_executable();
    let root = temp.path().join("sha256 repository");
    if !init_sha256_repository(&temp, &git, &root) {
        return;
    }
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["config", "user.name", "W04 fixture"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["config", "user.email", "w04@example.invalid"]),
    );
    let (service, opened, entry) = plan_committed_entry(
        &temp,
        &git,
        &root,
        "w04-blob-sha256-wrong",
        contents,
        SnapshotSelection::Head,
    )
    .await;
    let object_id = match entry.source() {
        SnapshotContentSource::GitBlob { object_id } => object_id,
        _ => panic!("the planned entry is a Git blob"),
    };
    // A second blob of the same five-byte length but different bytes,
    // written into this repository's object store by Git itself.
    let replacement_id = write_loose_blob(&git, &root, b"hallo");
    let replacement_path = root
        .join(".git")
        .join("objects")
        .join(&replacement_id[..2])
        .join(&replacement_id[2..]);
    let committed_path = blob_object_path(opened.root(), &entry);
    // Loose objects are stored read-only (0444): make the committed blob's
    // file writable, then overwrite it with the replacement's bytes.
    let mut writable = fs::metadata(&committed_path).unwrap().permissions();
    writable.set_mode(0o644);
    fs::set_permissions(&committed_path, writable).unwrap();
    fs::copy(&replacement_path, &committed_path).unwrap();
    // The premise: Git serves the swapped bytes under the committed object
    // id with no error, because loose-object reads do not re-hash.
    let served = try_run_git(&git, &root, &["cat-file", "blob", object_id.as_str()]);
    assert!(
        served.status.success(),
        "git cat-file refused the swapped loose object: {}",
        String::from_utf8_lossy(&served.stderr)
    );
    assert_eq!(served.stdout, b"hallo");
    // The capture verifies the served bytes against the planned object id
    // and refuses the mismatch through the public API.
    let mut sink = Vec::new();
    let refused = capture(&service, &opened, &entry, contents.len() as u64, &mut sink)
        .await
        .expect_err("a same-length wrong-byte blob refuses the capture");
    assert!(matches!(refused, SnapshotError::BlobDigestMismatch));
    // The partial-sink contract only promises that an error leaves the
    // sink holding possible partial bytes and the caller discards it, so
    // no content is asserted here.
    drop(sink);
}
