//! The `[job]` table: the job host's own settings, and the retirement of
//! `[coding]`.
//!
//! `[coding]` is not a table: every `[coding]` key is refused with the move to
//! `[job]` named rather than reported as an unknown field. `[jobs]` — a
//! run's budgets — sits one letter away, so a key written in the wrong one of
//! the two tables is refused naming the table it belongs to.

use std::path::Path;

use saya_config::{ConfigError, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

/// An absolute directory, on either platform: `program_dir` is refused unless
/// the canonical form does not depend on the working directory.
fn programs_dir() -> &'static str {
    if cfg!(windows) {
        "C:/saya-programs"
    } else {
        "/opt/saya-programs"
    }
}

fn user_with(toml: &str) -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(toml).expect("user fixture must parse"))
}

/// The contract of the rename: no `[coding]` key parses any more, and each one
/// names where its setting went — the `[job]` table, `[limits.job]`, `[agents]`
/// or `[tools]`. A retired key is not a typo, so serde's bare `unknown field`
/// is never the answer.
#[test]
fn no_coding_key_parses_and_each_names_its_destination() {
    for (toml, destination) in [
        ("[coding]\nenabled = true\n", "Delete the line"),
        ("[coding]\ncheck_profiles = {}\n", "Delete the section"),
        ("[coding]\nmax_concurrent_commands = 2\n", "Delete the key"),
        ("[coding]\nmax_children = 2\n", "agents.max_children"),
        (
            "[coding]\nmax_concurrent_model_requests = 2\n",
            "agents.max_concurrent_model_requests",
        ),
        ("[coding.agents.reviewer]\nrole = 'analyst'\n", "[agents]"),
        (
            "[coding]\nbudget_mode = 'bounded'\n",
            "limits.job.budget_mode",
        ),
        (
            "[coding]\nmax_logical_model_requests = 2\n",
            "limits.job.logical_requests",
        ),
        (
            "[coding]\nmax_physical_transport_attempts = 2\n",
            "limits.job.transport_attempts",
        ),
        ("[coding]\nmax_tool_calls = 2\n", "limits.job.tool_calls"),
        (
            "[coding]\nmax_known_tokens = 2\n",
            "limits.job.known_tokens",
        ),
        ("[coding]\nmax_job_tokens = 2\n", "limits.job.job_tokens"),
        (
            "[coding]\nmax_elapsed_seconds = 2\n",
            "limits.job.elapsed_seconds",
        ),
        ("[coding]\nshell = false\n", "[tools] allow"),
        (
            "[coding]\nprogram_dir = '/opt/programs'\n",
            "job.program_dir",
        ),
        (
            "[coding.extensions.rust]\nprotocol = 'lsp'\nprogram = 'rust-lsp'\nargv = []\nlanguage = 'rust'\n",
            "[job.extensions]",
        ),
        (
            "[coding]\nautomatic_lsp_profile = 'rust'\n",
            "job.automatic_lsp_profile",
        ),
    ] {
        let error = ConfigFile::from_toml(toml).expect_err("a retired key must fail loudly");
        assert!(
            matches!(error, ConfigError::Parse(_)),
            "a retired key is a parse failure: {error}"
        );
        let message = error.to_string();
        assert!(
            message.contains(destination),
            "the refusal must name {destination}: {message}"
        );
    }
}

/// A key the section never defined is refused by the section's own move: the
/// table itself is gone, so there is nothing left in `[coding]` to typo.
#[test]
fn a_coding_key_with_no_move_names_the_renamed_section() {
    let error = ConfigFile::from_toml("[coding]\ncoordinator_mode = 'direct'\n")
        .expect_err("the section is gone");
    let message = error.to_string();
    assert!(
        message.contains("[coding]") && message.contains("[job]"),
        "the refusal must name the renamed section and its new table: {message}"
    );
}

/// The three keys `[coding]` carried resolve from `[job]`, with the same
/// semantics: an absolute program directory, a named LSP profile, and the
/// extension profiles it selects from.
#[test]
fn the_job_table_carries_the_moved_settings() {
    let directory = programs_dir();
    let file = ConfigFile::from_toml(&format!(
        "[job]\nprogram_dir = '{directory}'\nautomatic_lsp_profile = 'rust'\n\n\
         [job.extensions.rust]\nprotocol = 'lsp'\nprogram = 'rust-lsp'\nargv = []\nlanguage = 'rust'\n"
    ))
    .expect("[job] must parse");
    assert_eq!(file.job.program_dir.as_deref(), Some(Path::new(directory)));
    assert_eq!(file.job.automatic_lsp_profile.as_deref(), Some("rust"));

    let resolved = resolve(user_with(&format!(
        "[job]\nprogram_dir = '{directory}'\nautomatic_lsp_profile = 'rust'\n\n\
         [job.extensions.rust]\nprotocol = 'lsp'\nprogram = 'rust-lsp'\nargv = []\nlanguage = 'rust'\n"
    )))
    .expect("resolution succeeds");
    assert_eq!(
        resolved.coding.program_dir.as_deref(),
        Some(Path::new(directory))
    );
    assert_eq!(
        resolved.coding.automatic_lsp_profile.as_deref(),
        Some("rust")
    );
    assert_eq!(
        resolved.coding.extensions["rust"].program(),
        "rust-lsp",
        "the extension profile resolves from [job.extensions]"
    );
}

/// `[jobs]` holds a run's budgets and `[job]` the job host's own settings. A
/// key written in the wrong one of the two is refused naming the table it
/// belongs to — the one-letter collision is a designed diagnostic, not an
/// unknown-field error that cannot say which table was meant.
#[test]
fn a_jobs_key_in_the_job_table_is_refused_naming_jobs() {
    for key in [
        "wall_clock_seconds = 60",
        "tokens_per_endpoint = 5",
        "turns = 3",
        "tool_calls = 4",
        "fetch = {}",
    ] {
        let error = ConfigFile::from_toml(&format!("[job]\n{key}\n"))
            .expect_err("a [jobs] key must not parse in [job]");
        let message = error.to_string();
        let name = key.split(' ').next().unwrap();
        assert!(
            message.contains("[jobs]") && message.contains(name),
            "the refusal must name [jobs] and the key {name}: {message}"
        );
    }
}

/// The same collision, read the other way: the job host's own settings are not
/// run budgets, and `[jobs]` refuses them naming `[job]`.
#[test]
fn a_job_key_in_the_jobs_table_is_refused_naming_job() {
    for key in [
        "program_dir = '/opt/programs'",
        "extensions = {}",
        "automatic_lsp_profile = 'rust'",
    ] {
        let error = ConfigFile::from_toml(&format!("[jobs]\n{key}\n"))
            .expect_err("a [job] key must not parse in [jobs]");
        let message = error.to_string();
        let name = key.split(' ').next().unwrap();
        assert!(
            message.contains("[job]") && message.contains(name),
            "the refusal must name [job] and the key {name}: {message}"
        );
    }
}

/// The guard names the two tables' own keys and nothing else: a key belonging
/// to neither is still the ordinary unknown-field error, and the one live
/// `[jobs]` setting still parses where it belongs.
#[test]
fn the_guard_leaves_other_keys_alone() {
    let error = ConfigFile::from_toml("[job]\nprogramme_dir = '/opt/programs'\n")
        .expect_err("a typo must fail");
    let message = error.to_string();
    assert!(
        message.contains("unknown field") && message.contains("programme_dir"),
        "a key of neither table keeps serde's own error: {message}"
    );
    ConfigFile::from_toml("[jobs.fetch]\nmax_file_bytes = 1024\n")
        .expect("[jobs.fetch] is still a run budget");
}
