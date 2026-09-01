use crate::common::*;
use std::fs;
use std::path::Path;

/// Runs the saya binary with every on-disk pointer isolated under `root`:
/// config home, state db, HOME (so the default state path can't reach the
/// developer's real `~/Library/Application Support`), and cwd. `saya_process`
/// only isolates config home; `ask` also opens a state db, so the ask-based
/// tests below need this fuller isolation.
fn saya_isolated(root: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(args)
        .current_dir(root)
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", root.join("home"))
        .output()
        .unwrap()
}

#[test]
fn config_init_creates_parseable_templates_with_stable_output() {
    // `config init` now writes the user layer (root/user-config/saya under
    // the test's SAYA_CONFIG_HOME), not.saya/. The success message names that
    // directory, so its exact bytes vary per run; the stable prefix and the
    // parseable/permissions/follow-up checks are what the invariant guards.
    let user_dir = |root: &std::path::Path| root.join("user-config/saya");
    for format in ["text", "json", "ndjson"] {
        let root = test_root(&format!("init-{format}"));
        let output = saya_process(&root, &["--format", format, "config", "init"]);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Created config.toml and connections.toml in"),
            "stable message prefix: {stdout}"
        );
        assert!(output.stderr.is_empty());

        let config = fs::read_to_string(user_dir(&root).join("config.toml")).unwrap();
        let connections = fs::read_to_string(user_dir(&root).join("connections.toml")).unwrap();
        saya_config::ConfigFile::from_toml(&config).unwrap();
        saya_config::ConnectionsFile::from_toml(&connections).unwrap();
        assert!(connections.contains("{ env = \"SAYA_ANALYTICS_PASSWORD\" }"));
        assert!(!config.contains("password = \""));

        // The rest of the CLI loads the created config. `connection list` and
        // `config show` do not need the secret. `config doctor` now exits
        // non-zero when the secret is unresolved, so resolve it to keep
        // this a "doctor reports a working post-init setup" check.
        assert!(
            saya_process(&root, &["connection", "list"])
                .status
                .success()
        );
        assert!(saya_process(&root, &["config", "show"]).status.success());
        let doctor = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
            .args(["config", "doctor"])
            .current_dir(&root)
            .env("SAYA_CONFIG_HOME", root.join("user-config"))
            .env("SAYA_ANALYTICS_PASSWORD", "a-resolved-secret")
            .output()
            .unwrap();
        assert!(doctor.status.success(), "stderr: {}", stderr(&doctor));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(user_dir(&root)).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for name in ["config.toml", "connections.toml"] {
                assert_eq!(
                    fs::metadata(user_dir(&root).join(name))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn config_init_refuses_overwrite_and_rolls_back_partial_creation() {
    let user_dir = |root: &std::path::Path| root.join("user-config/saya");
    let root = test_root("init-no-overwrite");
    let first = saya_process(&root, &["config", "init"]);
    assert!(first.status.success());
    let config_before = fs::read_to_string(user_dir(&root).join("config.toml")).unwrap();
    let second = saya_process(&root, &["config", "init"]);
    assert_eq!(second.status.code(), Some(2));
    assert!(stderr(&second).contains("already exists"));
    assert_eq!(
        fs::read_to_string(user_dir(&root).join("config.toml")).unwrap(),
        config_before
    );
    let _ = fs::remove_dir_all(&root);

    let root = test_root("init-rollback");
    fs::create_dir_all(user_dir(&root)).unwrap();
    fs::create_dir(user_dir(&root).join("connections.toml")).unwrap();
    let output = saya_process(&root, &["config", "init"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("connections.toml"));
    assert!(!user_dir(&root).join("config.toml").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn config_init_failures_use_stable_error_envelopes_without_path_leaks() {
    let user_dir = |root: &std::path::Path| root.join("user-config/saya");
    for (format, expected) in [
        ("text", "connections.toml already exists\n"),
        (
            "json",
            "{\"event\":\"error\",\"message\":\"connections.toml already exists\"}\n",
        ),
        (
            "ndjson",
            "{\"event\":\"error\",\"message\":\"connections.toml already exists\"}\n",
        ),
    ] {
        let root = test_root(&format!("init-failure-{format}"));
        fs::create_dir_all(user_dir(&root)).unwrap();
        fs::create_dir(user_dir(&root).join("connections.toml")).unwrap();
        let output = saya_process(&root, &["--format", format, "config", "init"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8_lossy(&output.stderr), expected);
        assert!(!stderr(&output).contains(&root.display().to_string()));
        assert!(!user_dir(&root).join("config.toml").exists());
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn discovery_prefers_project_over_user_and_explicit_env_file_is_opt_in() {
    let root = std::env::temp_dir().join(format!("saya-cli-discovery-{}", std::process::id()));
    let user = root.join("user");
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".saya")).unwrap();
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(user.join("config.toml"), "[ai]\nmodel = 'user'\n").unwrap();
    std::fs::write(
        project.join(".saya/config.toml"),
        "[ai]\nmodel = 'project'\n",
    )
    .unwrap();
    let options = saya_cli::GlobalOptions::default();
    let loaded =
        saya_cli::load_with_sources(&options, &project, &user, std::collections::BTreeMap::new())
            .unwrap();
    assert_eq!(loaded.resolved.ai.model, "project");
    std::fs::write(project.join(".env"), "SAYA_AI_MODEL = 'ignored'\n").unwrap();
    let without_opt_in =
        saya_cli::load_with_sources(&options, &project, &user, std::collections::BTreeMap::new())
            .unwrap();
    assert_eq!(without_opt_in.resolved.ai.model, "project");
    let env_file = project.join(".env.saya");
    std::fs::write(&env_file, "SAYA_AI_MODEL=explicit-env\n").unwrap();
    let explicit = saya_cli::GlobalOptions {
        env_file: Some(env_file),
        ..Default::default()
    };
    let with_opt_in = saya_cli::load_with_sources(
        &explicit,
        &project,
        &user,
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(with_opt_in.resolved.ai.model, "explicit-env");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn runtime_debug_redacts_merged_environment_values_and_keys() {
    let root = std::env::temp_dir().join(format!("saya-cli-debug-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let sentinel_key = "SAYA_TEST_SECRET_SENTINEL";
    let sentinel_value = "never-print-this-secret";
    let runtime = saya_cli::load_with_sources(
        &saya_cli::GlobalOptions::default(),
        &root,
        &root,
        std::collections::BTreeMap::from([(sentinel_key.into(), sentinel_value.into())]),
    )
    .unwrap();
    let diagnostic = format!("{runtime:?}");
    assert!(!diagnostic.contains(sentinel_key));
    assert!(!diagnostic.contains(sentinel_value));
    let _ = std::fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// a first run that ends somewhere. The cold path (empty config home, no
//.saya) used to land a new user on three unrelated failures, none naming a
// next step, and the third was self-inflicted: `config init` wrote only to the
// untrusted project layer, so the next command warned that it had ignored the
// templates init itself just wrote. These tests pin the post-fix behaviour:
// init writes the trusted user layer by default, `--project` keeps the old
// project-layer path, doctor advises (and exits non-zero when the setup cannot
// work), and the three failure paths each name an actionable next command.
// ---------------------------------------------------------------------------

/// Invariant 1 / deliverable 1: a default `config init` followed by any command
/// must NOT print "ignored N security-critical settings". Before the fix, init
/// wrote `.saya/` (the untrusted project layer), so the next command warned
/// about the very templates init had just written. Red before the fix; green
/// after, when init writes the trusted user layer.
#[test]
fn default_init_then_command_does_not_warn_about_ignored_settings() {
    let root = test_root("s18-init-no-warning");
    let init = saya_isolated(&root, &["config", "init"]);
    assert!(init.status.success(), "stderr: {}", stderr(&init));

    // The user-layer config must exist and the project layer must NOT.
    let user_config = root.join("user-config/saya/config.toml");
    assert!(user_config.exists(), "user config not written");
    assert!(root.join("user-config/saya/connections.toml").exists());
    assert!(
        !root.join(".saya").exists(),
        "default init must not write .saya/"
    );

    // Any command that loads config. `doctor` loads it and is itself part of
    // the slice, so it exercises the same load path the warning fires from.
    let doctor = saya_isolated(&root, &["config", "doctor"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert!(
        !combined.contains("ignored") && !combined.contains("security-critical"),
        "default init must not produce an ignored-settings warning: {combined}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Deliverable 2: `--project` writes the old `.saya/` pair, so the team-shared
/// project layer stays reachable (invariant 4). It is the untrusted layer, so a
/// command run afterward warns — that is the trust boundary doing its job, and
/// doctor explains how to apply the settings. This pins both halves.
#[test]
fn project_init_writes_saya_dir_and_the_next_command_warns() {
    let root = test_root("s18-init-project");
    let init = saya_isolated(&root, &["config", "init", "--project"]);
    assert!(init.status.success(), "stderr: {}", stderr(&init));
    assert!(root.join(".saya/config.toml").exists());
    assert!(root.join(".saya/connections.toml").exists());
    assert!(!root.join("user-config/saya/config.toml").exists());

    let doctor = saya_isolated(&root, &["config", "doctor"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert!(
        combined.contains("ignored") && combined.contains("security-critical"),
        "project init is the untrusted layer, so a command should warn: {combined}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Deliverable 2: the default-init success message points at the user config
/// directory, not `.saya/`, so a new user knows where their starter config
/// went. (Before the fix the message named `.saya/config.toml`.)
#[test]
fn default_init_message_names_the_user_config_directory() {
    let root = test_root("s18-init-message");
    let init = saya_isolated(&root, &["config", "init"]);
    assert!(init.status.success(), "stderr: {}", stderr(&init));
    let message = String::from_utf8_lossy(&init.stdout);
    assert!(
        !message.contains(".saya/"),
        "default init must not claim it wrote .saya/: {message}"
    );
    assert!(
        message.contains("user config") || message.contains("user-config"),
        "default init should name the user config directory: {message}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Q3 failure path 1 — provider unreachable: a configured-but-down AI endpoint.
/// The error must name an actionable next command. A user whose gateway is
/// momentarily down does NOT need to re-run init, so the next step is doctor
/// (and starting the provider), not init. Hermetic: a dead base_url is refused
/// regardless of whether ollama is running locally.
#[test]
fn ask_with_unreachable_provider_names_a_next_command() {
    let root = test_root("s18-ask-provider-down");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    // A base_url on a port nothing listens on: connection refused, fast, and
    // independent of any local ollama. No profile → the turn skips the registry
    // and calls the provider directly, which is the path we are testing.
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ai]\nbase_url = \"http://127.0.0.1:1\"\n",
    )
    .unwrap();
    let ask = saya_isolated(&root, &["--non-interactive", "ask", "count films"]);
    assert_eq!(ask.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&ask.stderr);
    assert!(
        stderr.contains("could not reach the provider"),
        "should report the unreachable provider: {stderr}"
    );
    assert!(
        stderr.contains("saya config doctor"),
        "provider-unreachable must name an actionable next command: {stderr}"
    );
    // Resist the wrong advice: a down gateway is not a missing config.
    assert!(
        !stderr.contains("config init"),
        "a down provider must not be told to re-run init: {stderr}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Q3 failure path 3 — unresolvable secret: a profile is configured but its
/// password reference does not resolve. This is "what is configured did not
/// work", not "nothing is configured", so the next step is setting the env var
/// (with doctor to list the unresolved references), not init.
#[test]
fn ask_with_unresolvable_secret_names_a_next_command() {
    let root = test_root("s18-ask-secret");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "default_profile = \"analytics\"\n[ai]\nbase_url = \"http://127.0.0.1:1\"\n",
    )
    .unwrap();
    fs::write(
        root.join("user-config/saya/connections.toml"),
        "[profiles.analytics]\ntype = \"postgresql\"\nhost = \"localhost\"\nport = 5432\n\
         database = \"warehouse\"\nuser = \"saya_readonly\"\n\
         password = { env = \"SAYA_ANALYTICS_PASSWORD\" }\nsslmode = \"require\"\n",
    )
    .unwrap();
    let ask = saya_isolated(&root, &["--non-interactive", "ask", "count films"]);
    assert_eq!(ask.status.code(), Some(5));
    let stderr = String::from_utf8_lossy(&ask.stderr);
    assert!(
        stderr.contains("secret reference") && stderr.contains("could not be resolved"),
        "should report the unresolved secret: {stderr}"
    );
    assert!(
        stderr.contains("saya config doctor"),
        "an unresolvable secret must name an actionable next command: {stderr}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Q3 failure path 2 / deliverable 4 — no config found. The empty-config
/// `config doctor` used to print "config file: not found" and exit 0, naming no
/// next step. It must now advise `saya config init` (nothing is configured) and
/// exit non-zero so a script can tell the setup is unusable.
#[test]
fn doctor_with_no_config_advises_init_and_exits_nonzero() {
    let root = test_root("s18-doctor-empty");
    let doctor = saya_isolated(&root, &["config", "doctor"]);
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(
        stdout.contains("config file: not found"),
        "doctor keeps the factual line: {stdout}"
    );
    assert!(
        stdout.contains("saya config init"),
        "doctor must advise an actionable next command when nothing is configured: {stdout}"
    );
    assert_ne!(
        doctor.status.code(),
        Some(0),
        "doctor must exit non-zero when the setup cannot work (Q4): {:?}",
        doctor.status.code()
    );
    let _ = fs::remove_dir_all(root);
}

/// Q4: doctor exits 0 when the setup can plausibly run a query (a selected
/// profile whose referenced secrets resolve), so the non-zero from the test
/// above means "broken", not "doctor ran". The env var resolves the template's
/// secret reference, so this is a genuinely working post-init setup.
#[test]
fn doctor_exits_zero_when_setup_can_work() {
    let root = test_root("s18-doctor-ok");
    let init = saya_isolated(&root, &["config", "init"]);
    assert!(init.status.success(), "stderr: {}", stderr(&init));
    let doctor = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["config", "doctor"])
        .current_dir(&root)
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", root.join("home"))
        .env("SAYA_ANALYTICS_PASSWORD", "a-resolved-secret")
        .output()
        .unwrap();
    assert_eq!(
        doctor.status.code(),
        Some(0),
        "a resolvable setup must exit 0: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr),
    );
    let _ = fs::remove_dir_all(root);
}
