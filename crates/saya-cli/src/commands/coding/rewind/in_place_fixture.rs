//! A real in-place coding job over a plain folder, shared by the rewind and
//! baseline-restore tests: a private temp root, a captured local baseline, and
//! the job's own private records.
#![cfg(unix)]

use crate::commands::coding::{
    CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob,
};
use crate::{GlobalOptions, load_with_sources};
use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::{CodingEventKind, CodingEventV1, JobId, JobState};
use std::{
    collections::BTreeMap,
    fs,
    num::NonZeroU64,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) struct Fixture {
    root: PathBuf,
    pub(super) folder: PathBuf,
    store: SqliteStateStore,
    pub(super) application: Arc<CodingApplication>,
    pub(super) job: Arc<NativeCodingJob>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) async fn fixture(case: &str, write_paths: Vec<String>) -> Fixture {
    fixture_with(case, write_paths, &[]).await
}

/// The same fixture with extra files captured into the baseline before the job
/// is created.
pub(super) async fn fixture_with(
    case: &str,
    write_paths: Vec<String>,
    extra: &[(&str, &[u8])],
) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-inplace-rewind-{case}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let root = root.canonicalize().unwrap();
    let folder = root.join("folder");
    fs::create_dir(&folder).unwrap();
    fs::create_dir(root.join("programs")).unwrap();
    fs::write(folder.join("tracked.txt"), b"original\n").unwrap();
    for (name, bytes) in extra {
        fs::write(folder.join(name), bytes).unwrap();
    }
    let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let runtime =
        load_with_sources(&GlobalOptions::default(), &folder, &root, BTreeMap::new()).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let application = CodingApplication::open_with_mode(
        runtime,
        store.clone(),
        &folder,
        &root.join("state"),
        &git,
        &root.join("programs"),
        crate::cli::RootMode::InPlace,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let request = CodingJobRequest::new(
        JobId::parse(&format!("in-place-rewind-{case}")).unwrap(),
        "edit the plain folder".into(),
        write_paths,
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    Fixture {
        root,
        folder,
        store,
        application,
        job,
    }
}

/// Records the first bytes a tool saw at `path`, as the job's own mutation
/// sites do; `None` records an absent path.
pub(super) fn record(fixture: &Fixture, path: &str, bytes: Option<&[u8]>, turn: u32) {
    let store = fixture
        .job
        .before_images
        .as_ref()
        .expect("an in-place job holds a before-image store");
    let path = saya_types::WorkspacePath::try_from(path.to_owned()).unwrap();
    store.record_first_touch(&path, bytes, false, turn).unwrap();
}

/// Ends the scripted job so a rewind is admitted: only `Running` refuses.
pub(super) async fn pause(fixture: &Fixture) {
    let mut cursor = None;
    let mut last = 0;
    loop {
        let page = fixture
            .store
            .list_coding_events(fixture.job.job_id(), cursor, 100)
            .await
            .unwrap();
        if let Some(event) = page.events.last() {
            last = event.sequence().get();
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let next = last + 1;
    fixture
        .store
        .append_coding_event(
            CodingEventV1::new(
                NonZeroU64::new(next).unwrap(),
                fixture.job.job_id().clone(),
                None,
                None,
                CodingEventKind::StateChanged {
                    state: JobState::Paused,
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
}

pub(super) fn write(fixture: &Fixture, name: &str, contents: &str) {
    fs::write(fixture.folder.join(name), contents).unwrap();
}

pub(super) fn read(fixture: &Fixture, name: &str) -> std::io::Result<String> {
    fs::read_to_string(fixture.folder.join(name))
}

/// Writes the job's private committed-mutation journal: one record per
/// `(path, contents, operation, turn)`, with the digest of `contents`.
pub(super) fn journal(fixture: &Fixture, records: &[(&str, &str, &str, u32)]) {
    let checkout = fixture.job.in_place_checkout().unwrap();
    let path = fixture
        .application
        .state_root
        .join(fixture.job.job_id().as_str())
        .join(format!(
            "known-commits-{}.jsonl",
            checkout.checkout_id().as_str()
        ));
    let mut lines = String::new();
    for (name, contents, operation, turn) in records {
        lines.push_str(&format!(
            "{}\n",
            serde_json::json!({
                "schema_version": 1,
                "operation": operation,
                "path": name,
                "committed_sha256": saya_types::sha256_hex(contents.as_bytes()),
                "size": contents.len(),
                "turn": turn,
            })
        ));
    }
    fs::write(path, lines).unwrap();
}
