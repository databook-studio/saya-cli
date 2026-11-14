use saya_agent::CancellationToken;
use saya_cli::{CodingApplication, GlobalOptions, RuntimeConfig, load_with_sources};
use saya_store::SqliteStateStore;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[path = "support_programs.rs"]
mod programs;
pub(crate) use programs::program_dir_for;

/// Load ceiling, not a product bound: how long a test waits for a scripted
/// event that the fixture will produce on its own (a run finishing, a provider
/// call arriving, a pause being acknowledged at the next turn boundary). It is
/// sized for a machine running the whole suite, and other test suites, in parallel,
/// where the same wait takes several times longer than it does alone. A wait
/// under this ceiling still ends the moment its condition holds.
pub(crate) const EVENT_WAIT: Duration = Duration::from_secs(180);

pub struct Fixture {
    pub root: PathBuf,
    pub repository: PathBuf,
    pub application: Arc<CodingApplication>,
    pub store: SqliteStateStore,
    pub personal_instructions: Option<PathBuf>,
    pub runtime: RuntimeConfig,
    pub git: PathBuf,
    pub program_dir: PathBuf,
}
struct Extras<'a> {
    personal_enabled: Option<bool>,
    extensions: Option<&'a str>,
    /// A trusted `[agents]` table, resolved through the same path the
    /// application uses, so a test can drive a configured helper type.
    agents: Option<&'a str>,
}

impl Extras<'_> {
    fn none() -> Self {
        Self {
            personal_enabled: None,
            extensions: None,
            agents: None,
        }
    }
}

impl Fixture {
    pub async fn new(label: &str) -> Self {
        Self::with_endpoint(label, None).await
    }
    pub async fn with_endpoint(label: &str, endpoint: Option<&str>) -> Self {
        Self::build(label, endpoint).await
    }
    pub async fn with_extensions(label: &str, endpoint: &str, extensions: &str) -> Self {
        Self::build_options(
            label,
            Some(endpoint),
            false,
            Extras {
                personal_enabled: None,
                extensions: Some(extensions),
                agents: None,
            },
            false,
            true,
        )
        .await
    }
    pub async fn with_personal_instructions(label: &str, endpoint: &str, enabled: bool) -> Self {
        Self::build_options(
            label,
            Some(endpoint),
            false,
            Extras {
                personal_enabled: Some(enabled),
                extensions: None,
                agents: None,
            },
            false,
            true,
        )
        .await
    }
    /// The endpoint fixture with a trusted `[agents]` registry, so a
    /// test can drive a configured helper type over the wire.
    pub async fn with_agents(label: &str, endpoint: &str, agents: &str) -> Self {
        Self::build_options(
            label,
            Some(endpoint),
            false,
            Extras {
                personal_enabled: None,
                extensions: None,
                agents: Some(agents),
            },
            false,
            true,
        )
        .await
    }
    async fn build(label: &str, endpoint: Option<&str>) -> Self {
        Self::build_options(label, endpoint, false, Extras::none(), false, true).await
    }
    pub async fn with_deadline(label: &str, endpoint: &str) -> Self {
        Self::build_options(label, Some(endpoint), true, Extras::none(), false, true).await
    }
    /// A plain folder with no Git repository: the in-place root every
    /// baseline test opens over.
    pub async fn in_place(label: &str) -> Self {
        Self::build_options(label, None, false, Extras::none(), true, false).await
    }
    /// The same plain folder, wired to a scripted model endpoint so a run can
    /// answer.
    pub async fn in_place_with_endpoint(label: &str, endpoint: &str) -> Self {
        Self::build_options(label, Some(endpoint), false, Extras::none(), true, false).await
    }
    /// A Git working tree opened in place: the folder has a real `.git`
    /// directory, but the job captures the filesystem baseline and creates no
    /// managed checkout.
    pub async fn in_place_git_with_endpoint(label: &str, endpoint: &str) -> Self {
        Self::build_options(label, Some(endpoint), false, Extras::none(), true, true).await
    }
    async fn build_options(
        label: &str,
        endpoint: Option<&str>,
        deadline: bool,
        extras: Extras<'_>,
        in_place: bool,
        init_git: bool,
    ) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "saya-native-{label}-{}-{stamp}",
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
        if init_git {
            for args in [
                &["init", "--quiet"][..],
                &["config", "user.name", "Native test"],
                &["config", "user.email", "native@example.invalid"],
            ] {
                git_run(&git, &repository, args);
            }
            fs::write(repository.join("tracked.txt"), b"original\n").unwrap();
            git_run(&git, &repository, &["add", "tracked.txt"]);
            git_run(&git, &repository, &["commit", "--quiet", "-m", "baseline"]);
        } else {
            fs::write(repository.join("tracked.txt"), b"original\n").unwrap();
        }
        let mut runtime = load_with_sources(
            &GlobalOptions::default(),
            &repository,
            &root,
            BTreeMap::new(),
        )
        .unwrap();
        if let Some(enabled) = extras.personal_enabled {
            runtime.resolved.instructions.personal_enabled = enabled;
        }
        if let Some(endpoint) = endpoint {
            runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
            runtime.resolved.ai.model = "native-fixture".into();
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
            endpoint_config.model = "native-fixture".into();
            endpoint_config.base_url = Some(endpoint.into());
            endpoint_config.api_key = None;
        }
        if deadline {
            // Setup jitter under a full-suite run reaches well past one second:
            // the job deadline must still fire after the provider is entered.
            // The transport's own timeouts sit far past it, so a loaded timer
            // cannot make the held request fail first and turn the graceful
            // budget stop into a run failure.
            runtime.resolved.ai.timeout_seconds = 30;
            runtime.resolved.ai.idle_timeout_seconds = 30;
            runtime.resolved.limits.job.elapsed_seconds =
                saya_config::BudgetLimit::Finite(std::num::NonZeroU64::new(4).unwrap());
        }
        if let Some(extensions) = extras.extensions {
            let file = saya_config::ConfigFile::from_toml(extensions).unwrap();
            let resolved_coding = saya_config::resolve(
                saya_config::ResolutionInput::new(saya_config::ConnectionsFile::default())
                    .with_user(file),
            )
            .unwrap()
            .coding;
            runtime.resolved.coding.extensions = resolved_coding.extensions;
            runtime.resolved.coding.automatic_lsp_profile = resolved_coding.automatic_lsp_profile;
        }
        if let Some(agents) = extras.agents {
            let file = saya_config::ConfigFile::from_toml(agents).unwrap();
            runtime.resolved.agents = saya_config::resolve(
                saya_config::ResolutionInput::new(saya_config::ConnectionsFile::default())
                    .with_user(file),
            )
            .unwrap()
            .agents;
        }
        let personal_instructions = extras
            .personal_enabled
            .map(|_| root.join("personal-instructions"));
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let extension_fixture = extras.extensions.is_some();
        let program_dir = if extension_fixture {
            program_dir_for(&["bash", "sh", "sleep"])
        } else if endpoint.is_some() {
            program_dir_for(&["sleep"])
        } else {
            program_dir_for(&["printf", "false"])
        };
        if extension_fixture {
            assert!(
                program_dir.join("sh").is_file(),
                "extension fixture requires a configured program directory containing sh"
            );
            assert!(
                program_dir.join("bash").is_file(),
                "post-edit LSP fixture requires a configured program directory containing bash"
            );
        }
        let application = CodingApplication::open_with_mode(
            runtime.clone(),
            store.clone(),
            &repository,
            &root.join("coding"),
            &git,
            program_dir.as_path(),
            if in_place {
                saya_cli::RootMode::InPlace
            } else {
                saya_cli::RootMode::Managed
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let application = match personal_instructions.as_deref() {
            Some(path) => application
                .with_personal_instruction_dir(Some(path))
                .unwrap(),
            None => application,
        };
        Self {
            root,
            repository,
            application,
            store,
            personal_instructions,
            runtime,
            git,
            program_dir: program_dir.to_path_buf(),
        }
    }
}
fn git_run(git: &Path, root: &Path, args: &[&str]) {
    let output = Command::new(git)
        .args(["-c", "core.hooksPath=/dev/null", "-C"])
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
