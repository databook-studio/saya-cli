use saya_agent::CancellationToken;
use saya_cli::{CodingApplication, GlobalOptions, load_with_sources};
use saya_store::SqliteStateStore;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
pub struct Fixture {
    pub root: PathBuf,
    pub repository: PathBuf,
    pub application: Arc<CodingApplication>,
    pub store: SqliteStateStore,
    pub personal_instructions: Option<PathBuf>,
}
impl Fixture {
    pub async fn new(label: &str) -> Self {
        Self::with_endpoint(label, None).await
    }
    pub async fn with_endpoint(label: &str, endpoint: Option<&str>) -> Self {
        Self::build(label, endpoint, false).await
    }
    pub async fn with_check(label: &str, endpoint: &str) -> Self {
        Self::build(label, Some(endpoint), true).await
    }
    pub async fn with_extensions(label: &str, endpoint: &str, extensions: &str) -> Self {
        Self::build_options(
            label,
            Some(endpoint),
            false,
            false,
            false,
            None,
            None,
            Some(extensions),
        )
        .await
    }
    pub async fn with_diagnostic_output(label: &str, output: &str) -> Self {
        Self::build_options(label, None, false, false, false, Some(output), None, None).await
    }
    pub async fn with_personal_instructions(label: &str, endpoint: &str, enabled: bool) -> Self {
        Self::build_options(
            label,
            Some(endpoint),
            false,
            false,
            false,
            None,
            Some(enabled),
            None,
        )
        .await
    }
    async fn build(label: &str, endpoint: Option<&str>, check: bool) -> Self {
        Self::build_options(label, endpoint, check, false, false, None, None, None).await
    }
    pub async fn with_deadline(label: &str, endpoint: &str) -> Self {
        Self::build_options(label, Some(endpoint), false, true, false, None, None, None).await
    }
    pub async fn with_slow_check(label: &str) -> Self {
        Self::build_options(label, None, true, false, true, None, None, None).await
    }
    async fn build_options(
        label: &str,
        endpoint: Option<&str>,
        check: bool,
        deadline: bool,
        slow: bool,
        diagnostic_output: Option<&str>,
        personal_enabled: Option<bool>,
        extensions: Option<&str>,
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
        let mut runtime = load_with_sources(
            &GlobalOptions::default(),
            &repository,
            &root,
            BTreeMap::new(),
        )
        .unwrap();
        runtime.resolved.coding.enabled = true;
        if let Some(enabled) = personal_enabled {
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
            runtime.resolved.coding.max_elapsed_seconds = 1;
        }
        if let Some(extensions) = extensions {
            let file = saya_config::ConfigFile::from_toml(extensions).unwrap();
            runtime.resolved.coding.extensions = saya_config::resolve(
                saya_config::ResolutionInput::new(saya_config::ConnectionsFile::default())
                    .with_user(file),
            )
            .unwrap()
            .coding
            .extensions;
        }
        let personal_instructions = personal_enabled.map(|_| root.join("personal-instructions"));
        if let Some(output) = diagnostic_output {
            let argument = serde_json::to_string(output).unwrap();
            let file = saya_config::ConfigFile::from_toml(&format!(
                "[coding.check_profiles.lint]\ncommand_id='diagnostic-check'\nprogram='printf'\nargv=[{argument}]\ntimeout_seconds=5\nenvironment_profile='confined'\ndiagnostic_format='cargo_json'\n"
            ))
            .unwrap();
            runtime.resolved.coding.check_profiles = saya_config::resolve(
                saya_config::ResolutionInput::new(saya_config::ConnectionsFile::default())
                    .with_user(file),
            )
            .unwrap()
            .coding
            .check_profiles;
        } else if check {
            let file=saya_config::ConfigFile::from_toml("[coding.check_profiles.lint]\ncommand_id='false-check'\nprogram='false'\nargv=[]\ntimeout_seconds=5\nenvironment_profile='confined'\n").unwrap();
            let file = if slow {
                saya_config::ConfigFile::from_toml("[coding.check_profiles.lint]\ncommand_id='held-check'\nprogram='sleep'\nargv=['10']\ntimeout_seconds=15\nenvironment_profile='confined'\n").unwrap()
            } else {
                file
            };
            runtime.resolved.coding.check_profiles = saya_config::resolve(
                saya_config::ResolutionInput::new(saya_config::ConnectionsFile::default())
                    .with_user(file),
            )
            .unwrap()
            .coding
            .check_profiles;
        }
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let extension_fixture = extensions.is_some();
        let program_dir = if extension_fixture {
            Path::new("/bin")
        } else {
            Path::new("/usr/bin")
        };
        if extension_fixture {
            assert!(
                program_dir.join("sh").is_file(),
                "extension fixture requires a configured program directory containing sh"
            );
        }
        let application = CodingApplication::open(
            runtime,
            store.clone(),
            &repository,
            &root.join("coding"),
            &git,
            program_dir,
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
