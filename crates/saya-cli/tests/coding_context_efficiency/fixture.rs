use saya_agent::CancellationToken;
use saya_cli::{
    CodingApplication, CodingAuthorization, CodingJobRequest, GlobalOptions, NativeCodingJob,
    load_with_sources,
};
use saya_store::SqliteStateStore;
use saya_types::JobId;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[path = "../coding_application/support_programs.rs"]
mod programs;

pub(super) struct Fixture {
    pub root: PathBuf,
    pub repository: PathBuf,
    pub application: Arc<CodingApplication>,
    pub store: SqliteStateStore,
}
impl Fixture {
    pub async fn new(endpoint: &str, budget: usize, window: Option<u64>, model: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "saya-context-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let root = root.canonicalize().unwrap();
        let repository = root.join("repo");
        fs::create_dir(&repository).unwrap();
        let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join("git"))
            .find(|path| path.is_file())
            .unwrap()
            .canonicalize()
            .unwrap();
        for args in [
            &["init", "--quiet"][..],
            &["config", "user.name", "Context test"],
            &["config", "user.email", "context@example.invalid"],
        ] {
            git_run(&git, &repository, args);
        }
        fs::write(repository.join("tracked.txt"), b"original\n").unwrap();
        for index in 0..16 {
            fs::write(
                repository.join(format!("padding-{index}.txt")),
                file_body(index),
            )
            .unwrap();
        }
        git_run(&git, &repository, &["add", "."]);
        git_run(&git, &repository, &["commit", "--quiet", "-m", "baseline"]);
        let mut runtime = load_with_sources(
            &GlobalOptions::default(),
            &repository,
            &root,
            BTreeMap::new(),
        )
        .unwrap();
        runtime.resolved.limits.job.logical_requests = finite(256);
        runtime.resolved.limits.job.tool_calls = finite(4096);
        // This fixture measures compaction across hundreds of deliberately
        // identical reads; raise the progress guards to their published
        // maximum so the run reaches the compaction path under test.
        runtime.resolved.limits.job.no_progress_turns = 10_000;
        runtime.resolved.limits.job.repeat_limit = 10_000;
        runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
        runtime.resolved.ai.model = "unknown-root-context-fixture".into();
        runtime.resolved.ai.base_url = Some(endpoint.into());
        runtime.resolved.ai.api_key = None;
        runtime.resolved.ai.retry_delays_ms.clear();
        runtime.resolved.ai.timeout_seconds = 10;
        runtime.resolved.ai.idle_timeout_seconds = 10;
        runtime.resolved.ai.context_byte_budget = budget;
        runtime.resolved.ai.context_window_tokens = window;
        runtime.resolved.ai.max_output_tokens = Some(4096);
        let effective = runtime
            .resolved
            .endpoints
            .get_mut(saya_config::ORCHESTRATOR_ROLE)
            .unwrap();
        effective.provider = saya_config::AiProvider::OpenaiCompatible;
        effective.model = model.into();
        effective.base_url = Some(endpoint.into());
        effective.api_key = None;
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let application = CodingApplication::open(
            runtime,
            store.clone(),
            &repository,
            &root.join("coding"),
            &git,
            &programs::program_dir_for(&["false"]),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        Self {
            root,
            repository,
            application,
            store,
        }
    }
    pub async fn create(&self) -> Arc<NativeCodingJob> {
        let request = CodingJobRequest::new(
            JobId::parse("context-job").unwrap(),
            "write for child-context; read actual files and review an owned candidate with a helper"
                .into(),
            vec!["new.txt".into()],
        )
        .unwrap();
        let approval =
            CodingAuthorization::scoped(&self.application.propose(&request).await.unwrap())
                .unwrap();
        self.application
            .create(request, &approval, CancellationToken::new())
            .await
            .unwrap()
    }
    pub fn file_body(&self, index: usize) -> String {
        file_body(index)
    }
}
fn file_body(index: usize) -> String {
    let label = format!("PRIVATE_FILE_BODY_{index:02}|");
    format!("{label}{}", "x".repeat(8192 - label.len()))
}
fn git_run(git: &Path, repository: &Path, args: &[&str]) {
    let output = Command::new(git)
        .args(["-c", "core.hooksPath=/dev/null", "-C"])
        .arg(repository)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Preserve failed fixtures for owner diagnostics; remove only this fresh successful fixture.
        if std::thread::panicking() {
            eprintln!("retained fixture {}", self.root.display());
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn finite(value: u64) -> saya_config::BudgetLimit {
    saya_config::BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}
