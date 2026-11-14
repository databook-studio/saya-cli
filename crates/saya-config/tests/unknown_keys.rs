//! Unknown-key rejection for configuration files.
//!
//! A typo'd key must fail loudly at parse time instead of silently falling
//! back to defaults. The security-critical case: `sslmodee = "verify-full"`
//! in a Postgres profile would otherwise silently drop TLS enforcement.

use saya_config::{ConfigError, ConfigFile, ConnectionsFile, ResolutionInput, resolve};
use std::path::Path;

#[test]
fn config_file_rejects_unknown_keys_with_the_key_named() {
    let error = ConfigFile::from_toml("[run]\nmax_rowz = 5\n").expect_err("typo must fail");
    assert!(
        error.to_string().contains("max_rowz"),
        "error must name the offending key: {error}"
    );
}

#[test]
fn config_file_still_accepts_known_sections_and_keys() {
    let config = ConfigFile::from_toml(
        "[run]\nmax_rows = 5\n[ai]\nmodel = 'm'\nallow_data_sharing = true\n[ui]\ntheme = 'dark'\n",
    )
    .expect("known keys must parse");
    assert_eq!(config.run.max_rows, Some(5));
}

#[test]
fn ui_section_rejects_unknown_keys_with_the_key_named() {
    let error = ConfigFile::from_toml("[ui]\ntheem = 'dark'\n").expect_err("typo must fail");
    assert!(
        error.to_string().contains("theem"),
        "error must name the offending key: {error}"
    );
}

#[test]
fn connections_rejects_unknown_profile_keys_per_backend() {
    let toml = r#"
[profiles.analytics]
type = "postgresql"
host = "localhost"
database = "db"
user = "u"
sslmodee = "verify-full"
"#;
    let error = ConnectionsFile::from_toml(toml).expect_err("typo'd sslmode must fail");
    assert!(
        error.to_string().contains("sslmodee"),
        "error must name the offending key: {error}"
    );
}

#[test]
fn connections_rejects_unknown_top_level_keys() {
    let error = ConnectionsFile::from_toml("[profils.x]\ntype = \"sqlite\"\npath = 'a.db'\n")
        .expect_err("unknown top-level table must fail");
    assert!(
        error.to_string().contains("profils"),
        "error must name the unknown section: {error}"
    );
}

#[test]
fn connections_still_accepts_every_documented_backend_key() {
    let toml = r#"
[profiles.pg]
type = "postgresql"
host = "h"
port = 5432
database = "d"
user = "u"
ssl_mode = "require"
password = { env = "PGPASS" }

[profiles.my]
type = "mysql"
host = "h"
database = "d"
user = "u"
ssl_ca = { file = "/tmp/ca.pem" }

[profiles.lite]
type = "sqlite"
path = "a.db"

[profiles.sf]
type = "snowflake"
account = "acme"
user = "u"
auth_type = "keypair"
private_key = { env = "PK" }
warehouse = "wh"
database = "db"
schema = "public"
role = "r"
"#;
    let profiles = ConnectionsFile::from_toml(toml).expect("documented keys must parse");
    assert_eq!(profiles.profiles.len(), 4);
}

/// A config file written by an older binary may still set the removed
/// `coding.coordinator_mode`. `[coding]` is not a section, so
/// such a file is refused by the section's own move rather than by a key it
/// never had — the same treatment any other key left in `[coding]` gets.
#[test]
fn a_removed_coding_coordinator_mode_key_names_the_renamed_section() {
    for value in ["delegate_only", "direct"] {
        let toml = format!("[coding]\ncoordinator_mode = \"{value}\"\n");
        let error = ConfigFile::from_toml(&toml).expect_err("a removed key must fail loudly");
        let message = error.to_string();
        assert!(
            matches!(error, ConfigError::Parse(_)),
            "a removed key is an ordinary parse failure: {message}"
        );
        assert!(
            message.contains("[coding]") && message.contains("[job]"),
            "the error must name the renamed section and its new table: {message}"
        );
    }
}

/// `[coding] enabled` is refused: coding is always available. A config that
/// still sets it must state the removal and name the fix, not report a bare
/// unknown field — a removed setting is not a typo.
#[test]
fn a_removed_coding_enabled_key_names_the_removal_and_the_fix() {
    for value in ["true", "false"] {
        let toml = format!("[coding]\nenabled = {value}\n");
        let error = ConfigFile::from_toml(&toml).expect_err("a removed key must fail loudly");
        let message = error.to_string();
        assert!(
            matches!(error, ConfigError::Parse(_)),
            "a removed key is an ordinary parse failure: {message}"
        );
        assert!(
            message.contains(
                "`[coding] enabled` was removed; coding is always available. Delete the line."
            ),
            "the error must state the removal and the fix: {message}"
        );
    }
}

/// `[coding] check_profiles` is refused: checks do not run. Both the
/// nested-table form and the inline form must state the removal and name the
/// fix, not report a bare unknown field — a removed setting is not a typo.
#[test]
fn a_removed_coding_check_profiles_key_names_the_removal_and_the_fix() {
    for toml in [
        "[coding.check_profiles.lint]\ncommand = \"cargo clippy\"\n",
        "[coding]\ncheck_profiles = {}\n",
    ] {
        let error = ConfigFile::from_toml(toml).expect_err("a removed key must fail loudly");
        let message = error.to_string();
        assert!(
            matches!(error, ConfigError::Parse(_)),
            "a removed key is an ordinary parse failure: {message}"
        );
        assert!(
            message.contains(
                "[coding] check_profiles was removed; checks no longer run. Delete the section."
            ),
            "the error must state the removal and the fix: {message}"
        );
    }
}

/// `[jobs.runner]` is refused: `run_program` does not exist, and the one
/// key that outlived it (`program_dir`) moved to `[job]`. Both the
/// nested-table form and the inline form must state the removal and name the
/// fix, not report a bare unknown field — a removed setting is not a typo.
#[test]
fn a_removed_jobs_runner_table_names_the_removal_and_the_fix() {
    for toml in [
        "[jobs.runner]\nallow = [\"jq\"]\nprogram_dir = \"/opt/programs\"\n",
        "[jobs]\nrunner = {}\n",
    ] {
        let error = ConfigFile::from_toml(toml).expect_err("a removed section must fail loudly");
        let message = error.to_string();
        assert!(
            matches!(error, ConfigError::Parse(_)),
            "a removed section is an ordinary parse failure: {message}"
        );
        assert!(
            message.contains(
                "[jobs.runner] was removed; run_program no longer exists. Move program_dir to \
                 [job] program_dir and delete the section."
            ),
            "the error must state the removal and the fix: {message}"
        );
    }
}

/// `[jobs.interpreter]` is refused along with the interpreter capability. Both
/// forms must state the removal and name the fix.
#[test]
fn a_removed_jobs_interpreter_table_names_the_removal_and_the_fix() {
    for toml in [
        "[jobs.interpreter]\nallow = [\"python3\"]\n",
        "[jobs]\ninterpreter = {}\n",
    ] {
        let error = ConfigFile::from_toml(toml).expect_err("a removed section must fail loudly");
        let message = error.to_string();
        assert!(
            matches!(error, ConfigError::Parse(_)),
            "a removed section is an ordinary parse failure: {message}"
        );
        assert!(
            message.contains(
                "[jobs.interpreter] was removed; run_program no longer exists. Delete the section."
            ),
            "the error must state the removal and the fix: {message}"
        );
    }
}

/// `[coding] max_concurrent_commands` is refused: owner commands do not
/// run, so the key has no consumer. A config that still sets it must state
/// the removal and name the fix.
#[test]
fn a_removed_coding_max_concurrent_commands_key_names_the_removal_and_the_fix() {
    let error = ConfigFile::from_toml("[coding]\nmax_concurrent_commands = 2\n")
        .expect_err("a removed key must fail loudly");
    let message = error.to_string();
    assert!(
        matches!(error, ConfigError::Parse(_)),
        "a removed key is an ordinary parse failure: {message}"
    );
    assert!(
        message.contains(
            "[coding] max_concurrent_commands was removed; owner commands no longer run. \
             Delete the key."
        ),
        "the error must state the removal and the fix: {message}"
    );
}

/// `program_dir` outlived `[jobs.runner]`: it is the trusted directory the
/// job host and its extensions launch programs from, so it lives in
/// `[job] program_dir` with the same semantics — absolute, optional, and
/// carried into the resolved diagnostics under `job_program_dir`.
#[test]
fn job_program_dir_resolves_and_is_reported_under_its_own_key() {
    let directory = if cfg!(windows) {
        "C:/saya-programs"
    } else {
        "/opt/saya-programs"
    };
    let file = ConfigFile::from_toml(&format!("[job]\nprogram_dir = '{directory}'\n"))
        .expect("[job] program_dir must parse");
    assert_eq!(file.job.program_dir.as_deref(), Some(Path::new(directory)));

    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(file.clone()))
            .expect("resolution succeeds");
    assert_eq!(
        resolved.coding.program_dir.as_deref(),
        Some(Path::new(directory)),
        "the declared directory resolves"
    );
    assert_eq!(
        file.redacted_diagnostics().job_program_dir.as_deref(),
        Some(directory),
        "the declared directory is reported under job_program_dir"
    );
    assert_eq!(
        resolved.redacted_diagnostics().job_program_dir.as_deref(),
        Some(directory),
        "the effective directory is reported under job_program_dir"
    );
    assert_eq!(
        resolved
            .provenance
            .get("job.program_dir")
            .map(|origin| origin.source.clone()),
        Some(saya_config::SettingSource::UserFile),
        "the key records its real origin"
    );
}

/// The directory must be absolute, exactly as it had to be under
/// `[jobs.runner]`: the canonical form must not depend on the working
/// directory the config was loaded from.
#[test]
fn a_relative_job_program_dir_is_rejected_naming_the_field() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[job]\nprogram_dir = 'programs'\n").unwrap()),
    )
    .expect_err("a relative directory must be a typed resolve error");
    let message = error.to_string();
    assert!(
        message.contains("job.program_dir") && message.contains("absolute"),
        "the error must name the field and the requirement: {message}"
    );
}

#[test]
fn parse_errors_are_typed_as_config_parse_failures() {
    let error = ConfigFile::from_toml("[run]\nnope = 1\n").unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));
}

// The package-owned copies are the canonical inputs for these tests. The
// workspace-root distribution copies are checked for byte equality by the
// crate package inspector.
fn example(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

#[test]
fn shipped_connections_example_parses_end_to_end() {
    let file = ConnectionsFile::from_toml(&example("connections.example.toml"))
        .expect("examples/connections.toml must parse");
    assert_eq!(file.profiles.len(), 6);
}

#[test]
fn shipped_connections_docker_example_parses_end_to_end() {
    let file = ConnectionsFile::from_toml(&example("connections.docker.example.toml"))
        .expect("examples/connections.docker.toml must parse");
    assert_eq!(file.profiles.len(), 2);
}

// Invariant: the accepted key set is owned by the type, once. serde's
// `deny_unknown_fields` enforces it directly, so a rejected key's error names
// the type's own fields (`expected one of... <a declared field>...`).
// A reintroduced hand-written shadow list would either stop rejecting unknown
// keys (if it replaced serde) or carry a different message (`unknown key... in
// profile...`) — in either case this assertion, which pins the serde-shaped
// message and a declared Postgres field, breaks. That breakage is the alarm:
// it means the key set is no longer defined by the type alone.
#[test]
fn rejected_profile_key_error_names_the_types_own_fields() {
    let toml = r#"
[profiles.analytics]
type = "postgresql"
host = "localhost"
database = "db"
user = "u"
sslmodee = "verify-full"
"#;
    let error = ConnectionsFile::from_toml(toml).expect_err("typo'd sslmode must fail");
    let message = error.to_string();
    assert!(
        message.contains("unknown field"),
        "serde must reject the unknown key: {message}"
    );
    assert!(
        message.contains("expected one of"),
        "the rejected set must come from the type, not a shadow list: {message}"
    );
    // `password` is a declared Postgres field. If it disappears from the
    // expected list, the type and the error have drifted apart.
    assert!(
        message.contains("password"),
        "the expected list must reflect the type's fields: {message}"
    );
}
