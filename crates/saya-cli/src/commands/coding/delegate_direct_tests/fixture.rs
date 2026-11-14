//! Shared fixture and helpers for the delegation admission tests.
//!
//! Helpers share W and the assignment cap is 32 KiB. The fixture
//! builds one approved job over a fresh temporary git repository; the sibling
//! test modules reach the same job shape through it.

use super::{
    CodingApplication, CodingAuthorization, CodingJobRequest, CodingTask, NativeCodingJob,
};
use crate::{GlobalOptions, load_with_sources};
use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::{AgentId, JobId};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn task(agent_id: &str) -> CodingTask {
    CodingTask {
        agent_id: AgentId::parse(agent_id).unwrap(),
        subagent_type: "general".into(),
        task: format!("write for {agent_id}"),
        write_paths: Vec::new(),
        dependencies: vec![],
    }
}

pub(super) fn sized_task(agent_id: &str, bytes: usize) -> CodingTask {
    let lead = format!("write for {agent_id}; ");
    let task = format!("{lead}{}", "x".repeat(bytes.saturating_sub(lead.len())));
    CodingTask {
        agent_id: AgentId::parse(agent_id).unwrap(),
        task,
        subagent_type: "general".into(),
        write_paths: Vec::new(),
        dependencies: vec![],
    }
}

#[cfg(unix)]
pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) job: Arc<NativeCodingJob>,
}

#[cfg(unix)]
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
pub(super) async fn fixture(case: &str, write_paths: Vec<String>) -> Fixture {
    fixture_with(case, write_paths, None, None, None).await
}

/// The same fixture whose job carries the `[agents]` table it states — the
/// registry and the two bounds — instead of the built-ins, so a test can drive
/// a configured helper type through admission and composition.
#[cfg(unix)]
pub(super) async fn fixture_with_agents(case: &str, agents_toml: &str) -> Fixture {
    fixture_with(
        case,
        vec!["tracked.txt".into()],
        None,
        None,
        Some(agents_toml),
    )
    .await
}

/// The same fixture with its model endpoint pointed at `endpoint` (a local
/// scripted provider) and, when given, a `coding.max_children` bound, so a
/// test can drive the job's children through a controllable transport.
#[cfg(unix)]
pub(super) async fn fixture_with_endpoint(
    case: &str,
    write_paths: Vec<String>,
    endpoint: &str,
    max_children: Option<usize>,
) -> Fixture {
    fixture_with(case, write_paths, Some(endpoint), max_children, None).await
}

/// The endpoint fixture with an `[agents]` table as well, so a test can drive
/// a configured helper type — and the child bound the same table states —
/// through the whole direct-mode path.
#[cfg(unix)]
pub(super) async fn fixture_with_endpoint_and_agents(
    case: &str,
    write_paths: Vec<String>,
    endpoint: &str,
    agents_toml: &str,
) -> Fixture {
    fixture_with(case, write_paths, Some(endpoint), None, Some(agents_toml)).await
}

#[cfg(unix)]
async fn fixture_with(
    case: &str,
    write_paths: Vec<String>,
    endpoint: Option<&str>,
    max_children: Option<usize>,
    agents_toml: Option<&str>,
) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-delegate-direct-{case}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = root.join("repo");
    fs::create_dir(&repository).unwrap();
    fs::create_dir(root.join("programs")).unwrap();
    let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    for args in [
        &["init", "--quiet"][..],
        &["config", "user.name", "Delegate direct proof"],
        &["config", "user.email", "delegate-direct@example.invalid"],
    ] {
        git_run(&git, &repository, args);
    }
    fs::write(repository.join("tracked.txt"), b"original\n").unwrap();
    git_run(&git, &repository, &["add", "tracked.txt"]);
    git_run(&git, &repository, &["commit", "--quiet", "-m", "baseline"]);
    let mut runtime = load_with_sources(
        &GlobalOptions::default(),
        &repository,
        &root,
        BTreeMap::new(),
    )
    .unwrap();
    if let Some(endpoint) = endpoint {
        runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
        runtime.resolved.ai.model = "delegate-direct-fixture".into();
        runtime.resolved.ai.base_url = Some(endpoint.into());
        runtime.resolved.ai.api_key = None;
        runtime.resolved.ai.retry_delays_ms.clear();
        runtime.resolved.ai.timeout_seconds = 5;
        runtime.resolved.ai.idle_timeout_seconds = 5;
        let endpoint_config = runtime
            .resolved
            .endpoints
            .get_mut(saya_config::ORCHESTRATOR_ROLE)
            .unwrap();
        endpoint_config.provider = saya_config::AiProvider::OpenaiCompatible;
        endpoint_config.model = "delegate-direct-fixture".into();
        endpoint_config.base_url = Some(endpoint.into());
        endpoint_config.api_key = None;
    }
    // The `[agents]` table is read the way a user file is, so the bound a test
    // states in it — written out here when the test states only the number — is
    // the bound the scheduler opens the job under, through the config path and
    // not as a value poked into the resolved struct.
    let agents_toml = agents_toml.map(str::to_owned).or_else(|| {
        max_children.map(|max_children| format!("[agents]\nmax_children = {max_children}\n"))
    });
    if let Some(toml) = agents_toml {
        let declared =
            saya_config::ConfigFile::from_toml(&toml).expect("the agents fixture parses");
        runtime.resolved.agents = saya_config::resolve(
            saya_config::ResolutionInput::new(saya_config::ConnectionsFile::default())
                .with_user(declared),
        )
        .expect("the agents fixture resolves")
        .agents;
    }
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let application = CodingApplication::open(
        runtime,
        store,
        &repository,
        &root.join("state"),
        &git,
        &root.join("programs"),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let request = CodingJobRequest::new(
        JobId::parse(&format!("delegate-direct-{case}")).unwrap(),
        "exercise direct-mode delegation admission".into(),
        write_paths,
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    Fixture { root, job }
}

#[cfg(unix)]
pub(super) fn git_run(git: &Path, repository: &Path, args: &[&str]) {
    let output = Command::new(git)
        .args(["-c", "core.hooksPath=/dev/null", "-C"])
        .arg(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
