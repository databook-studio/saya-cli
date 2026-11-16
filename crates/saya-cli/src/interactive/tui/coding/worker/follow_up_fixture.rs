use super::follow_up_provider::Provider;
use crate::{
    CodingApplication, CodingAuthorization, CodingJobRequest, GlobalOptions, RuntimeConfig,
    load_with_sources,
};
use saya_agent::{AgentOutput, CancellationToken, NoopEventSink};
use saya_store::{CodingCompletion, SqliteStateStore};
use saya_types::JobId;
use std::sync::Arc;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
pub(crate) struct Fixture {
    pub repository: PathBuf,
    pub runtime: RuntimeConfig,
    pub store: SqliteStateStore,
    pub completion: CodingCompletion,
    pub job: Arc<crate::NativeCodingJob>,
    pub output: AgentOutput,
}
impl Fixture {
    pub async fn new(root: &Path, provider: &Provider) -> Self {
        Self::with_paths(root, provider, vec!["tracked.txt".into()]).await
    }
    pub async fn with_paths(root: &Path, provider: &Provider, paths: Vec<String>) -> Self {
        let repository = root.join("repo");
        fs::create_dir(&repository).unwrap();
        for args in [
            &["init", "--quiet"][..],
            &["config", "user.name", "Worker fixture"],
            &["config", "user.email", "worker@example.invalid"],
        ] {
            git(&repository, args);
        }
        fs::write(repository.join("tracked.txt"), b"original\n").unwrap();
        git(&repository, &["add", "tracked.txt"]);
        git(&repository, &["commit", "--quiet", "-m", "baseline"]);
        let programs = root.join("programs");
        fs::create_dir(&programs).unwrap();
        let mut runtime = load_with_sources(
            &GlobalOptions::default(),
            &repository,
            root,
            BTreeMap::new(),
        )
        .unwrap();
        runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
        runtime.resolved.ai.model = "worker-fixture".into();
        runtime.resolved.ai.base_url = Some(provider.endpoint.clone());
        runtime.resolved.ai.api_key = None;
        runtime.resolved.ai.retry_delays_ms.clear();
        runtime.resolved.ai.timeout_seconds = 30;
        runtime.resolved.ai.idle_timeout_seconds = 30;
        runtime.resolved.coding.program_dir = Some(programs);
        let endpoint = runtime
            .resolved
            .endpoints
            .get_mut(saya_config::ORCHESTRATOR_ROLE)
            .unwrap();
        endpoint.provider = saya_config::AiProvider::OpenaiCompatible;
        endpoint.model = "worker-fixture".into();
        endpoint.base_url = Some(provider.endpoint.clone());
        endpoint.api_key = None;
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let application = CodingApplication::session(
            runtime.clone(),
            store.clone(),
            &repository,
            crate::cli::RootMode::Managed,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let request = CodingJobRequest::new(
            JobId::parse("worker-follow-up").unwrap(),
            "Prepare the bounded owned child with a helper".into(),
            paths,
        )
        .unwrap();
        let approval =
            CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
        let job = application
            .create(request, &approval, CancellationToken::new())
            .await
            .unwrap();
        let output = tokio::time::timeout(Duration::from_secs(30), job.run(&NoopEventSink))
            .await
            .unwrap()
            .unwrap();
        let completion = store
            .list_coding_completion_history(job.job_id(), None, 32)
            .await
            .unwrap()
            .items
            .pop()
            .unwrap();
        Self {
            repository,
            runtime,
            store,
            completion,
            job,
            output,
        }
    }
}
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null", "-C"])
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
