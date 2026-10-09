/// A default `config init` followed by any command
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

/// `--project` writes the old `.saya/` pair, so the team-shared
/// project layer stays reachable. It is the untrusted layer, so a
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

/// The default-init success message points at the user config
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

/// Failure path — provider unreachable: a configured-but-down AI endpoint.
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

/// Failure path — unresolvable secret: a profile is configured but its
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

/// Failure path — no config found. The empty-config
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

/// Doctor exits 0 when the setup can plausibly run a query (a selected
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

/// `--candidates` reaches the resolved config end to end: the flag is wired
/// into `CliOverrides` and then into `ResolvedConfig.candidates`, not just
/// accepted and discarded. Guards against the "advertised but unread"
/// regression this crate's `config show` flags once had.
#[test]
fn candidates_flag_reaches_the_resolved_config() {
    let root = test_root("saya-cli-candidates-wiring");
    let loaded = saya_cli::load_with_sources(
        &saya_cli::GlobalOptions {
            candidates: Some(3),
            ..Default::default()
        },
        &root,
        &root,
        std::collections::BTreeMap::new(),
    )
    .expect("resolution succeeds");
    assert_eq!(loaded.resolved.candidates, 3);
    let _ = fs::remove_dir_all(&root);
}
