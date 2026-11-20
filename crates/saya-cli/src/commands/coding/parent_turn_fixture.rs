//! The in-place job and the managed job the turn-per-message tests share.
//!
//! The in-place fixture states no write scope: a job that does keeps its own
//! readiness precompute alive in this process, and the folder lease with it.

use super::super::direct_root_fixture;
use super::super::{CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob};
use crate::{GlobalOptions, load_with_sources};
use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::JobId;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) job: Arc<NativeCodingJob>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// An in-place job over a plain folder, wired to `endpoint`.
pub(super) async fn in_place(case: &str, endpoint: &str) -> Fixture {
    in_place_with(case, endpoint, None).await
}

/// The same, with the run's context byte budget stated by the test — the one
/// bound the shared history assembly reads, so a test can make a run's
/// history outgrow it and see which turns the bound drops.
pub(super) async fn in_place_with_budget(
    case: &str,
    endpoint: &str,
    context_byte_budget: usize,
) -> Fixture {
    in_place_with(case, endpoint, Some(context_byte_budget)).await
}

async fn in_place_with(case: &str, endpoint: &str, byte_budget: Option<usize>) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-turn-{case}-{}-{stamp}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let root = root.canonicalize().unwrap();
    let folder = root.join("repo");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("tracked.txt"), b"original\n").unwrap();
    let programs = root.join("programs");
    fs::create_dir(&programs).unwrap();
    let git = git_path();
    let mut runtime = load_with_sources(&GlobalOptions::default(), &folder, &root, BTreeMap::new())
        .expect("the in-place fixture resolves");
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.model = "turn-per-message".into();
    runtime.resolved.ai.base_url = Some(endpoint.into());
    runtime.resolved.ai.api_key = None;
    runtime.resolved.ai.retry_delays_ms.clear();
    runtime.resolved.ai.timeout_seconds = 5;
    runtime.resolved.ai.idle_timeout_seconds = 5;
    runtime.resolved.ai.max_output_tokens = Some(4096);
    if let Some(byte_budget) = byte_budget {
        runtime.resolved.ai.context_byte_budget = byte_budget;
    }
    let effective = runtime
        .resolved
        .endpoints
        .get_mut(saya_config::ORCHESTRATOR_ROLE)
        .unwrap();
    effective.provider = saya_config::AiProvider::OpenaiCompatible;
    effective.model = "turn-per-message".into();
    effective.base_url = Some(endpoint.into());
    effective.api_key = None;
    let application = CodingApplication::open_in_place(
        runtime,
        SqliteStateStore::new(root.join("state.sqlite3")),
        &folder,
        &root.join("state"),
        &git,
        &programs,
        &CancellationToken::new(),
    )
    .await
    .expect("the in-place application opens");
    let request = CodingJobRequest::new(
        JobId::parse(&format!("turn-{case}")).unwrap(),
        "first message: read this folder".into(),
        Vec::new(),
    )
    .unwrap();
    let approval = CodingAuthorization::scoped(
        &application
            .propose(&request)
            .await
            .expect("the fixture's own request proposes"),
    )
    .unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .expect("the in-place fixture job is created");
    Fixture { root, job }
}

/// A managed-checkout job: the shape this release runs one turn per run.
pub(super) async fn managed(case: &str) -> Fixture {
    let base = direct_root_fixture::fixture(case, vec!["tracked.txt".into()]).await;
    let root = base.root.clone();
    let job = Arc::clone(&base.job);
    // This fixture owns the cleanup; forget `base` so its Drop does not remove
    // the directory out from under it.
    std::mem::forget(base);
    Fixture { root, job }
}

fn git_path() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap()
}
