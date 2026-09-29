//! Unit tests for the investigation flag grammar's usage errors (D2/D8): an
//! unknown flag-shaped token is echoed only up to (not including) its first
//! `=` — a refused token's value never reaches the transcript or the saved
//! session — and the attached `--param=` spelling adds the spelling hint. A
//! token without `=` echoes whole, as before.

use super::{scan, unknown_flag_error};
use crate::slash::SlashParseError;

const RUN_USAGE: &str = " (usage: /investigation run <id>)";

fn scan_run(tail: &str) -> Result<(), SlashParseError> {
    scan(
        tail,
        &["--connection"],
        &["--revalidate"],
        &["--param"],
        RUN_USAGE,
    )
    .map(|_| ())
}

/// The audit's R061-2 line, at the parser: the refusal names the token's
/// shape (`--param=…`), never its value, and hints at the accepted spelling.
#[test]
fn an_unknown_attached_param_token_echoes_without_its_value() {
    let error = scan_run("run abc --param=label=two confidential words").unwrap_err();
    assert!(
        error.0.contains("unknown investigation flag: --param=…"),
        "the echo stops before the value: {}",
        error.0
    );
    assert!(
        error.0.contains("use --param name=value"),
        "the attached spelling is hinted: {}",
        error.0
    );
    assert!(error.0.contains(RUN_USAGE), "the usage stays: {}", error.0);
    assert!(
        !error.0.contains("label=two"),
        "the value head must not echo: {}",
        error.0
    );
    assert!(
        !error.0.contains("confidential words"),
        "the value tail must not echo: {}",
        error.0
    );
}

/// The bare attached spelling (`--param=` with an empty binding) gets the
/// same echo and hint.
#[test]
fn a_bare_attached_param_token_echoes_with_the_hint() {
    let error = scan_run("run abc --param=").unwrap_err();
    assert!(
        error.0.contains("unknown investigation flag: --param=…")
            && error.0.contains("use --param name=value"),
        "the empty attached binding is hinted: {}",
        error.0
    );
}

/// A non-param unknown flag that carries a value (`--connection=demo`, a
/// typo'd spelling of a known flag) hides the value too, without the
/// param-specific hint.
#[test]
fn an_unknown_flag_with_a_value_echoes_without_the_value() {
    let error = scan_run("run abc --connection=demo --revalidate").unwrap_err();
    assert!(
        error
            .0
            .contains("unknown investigation flag: --connection=…"),
        "the echo stops before the value: {}",
        error.0
    );
    assert!(
        !error.0.contains("demo"),
        "the value must not echo: {}",
        error.0
    );
    assert!(
        !error.0.contains("use --param name=value"),
        "the hint is param-specific: {}",
        error.0
    );
}

/// An unknown flag without `=` echoes whole, as before.
#[test]
fn an_unknown_flag_without_a_value_echoes_whole() {
    let error = scan_run("run abc --bogus").unwrap_err();
    assert!(
        error.0.contains("unknown investigation flag: --bogus"),
        "the token echoes as today: {}",
        error.0
    );
}

/// The same truncation on the outside-a-zone unknown-flag site: a flag-shaped
/// token after a boolean is refused with its value hidden.
#[test]
fn an_unknown_flag_after_a_boolean_echoes_without_its_value() {
    let error = scan_run("run abc --revalidate --bogus=1").unwrap_err();
    assert!(
        error.0.contains("unknown investigation flag: --bogus=…"),
        "the echo stops before the value: {}",
        error.0
    );
    assert!(
        !error.0.contains("--bogus=1"),
        "the value must not echo: {}",
        error.0
    );
}

/// The dispatcher's unknown-subcommand refusal: a flag-shaped token there
/// (the subcommand was forgotten) echoes without its value, with the hint.
#[test]
fn an_unknown_flag_shaped_subcommand_token_echoes_without_its_value() {
    let error = super::super::parse_investigation_command("investigation", "--param=x=y run abc")
        .unwrap_err();
    assert!(
        error
            .0
            .contains("unknown investigation subcommand: --param=…"),
        "the echo stops before the value: {}",
        error.0
    );
    assert!(
        !error.0.contains("x=y"),
        "the value must not echo: {}",
        error.0
    );
}

/// A non-flag unknown subcommand echoes as today, without the hint.
#[test]
fn an_unknown_subcommand_without_a_value_echoes_whole() {
    let error = super::super::parse_investigation_command("investigation", "saev x").unwrap_err();
    assert!(
        error.0.contains("unknown investigation subcommand: saev")
            && !error.0.contains("use --param name=value"),
        "the token echoes as today, no param hint: {}",
        error.0
    );
}

/// The export parser's own unknown-flag refusal truncates the same way.
#[test]
fn export_refuses_an_unknown_flag_without_its_value() {
    let error =
        super::super::parse_investigation_command("investigation", "export abc --param=x=y")
            .unwrap_err();
    assert!(
        error.0.contains("unknown investigation flag: --param=…")
            && error.0.contains("use --param name=value"),
        "the export refusal truncates and hints: {}",
        error.0
    );
    assert!(
        !error.0.contains("x=y"),
        "the value must not echo: {}",
        error.0
    );
}

/// The helper's shape directly: truncation at the first `=`, hint only for
/// the attached `--param=` spelling.
#[test]
fn the_error_helper_truncates_at_the_first_equals() {
    let flag = unknown_flag_error("investigation flag", "--param=a=b --more", RUN_USAGE).0;
    assert_eq!(
        flag,
        format!("unknown investigation flag: --param=…{RUN_USAGE}; use --param name=value")
    );
    let plain = unknown_flag_error("investigation flag", "--connection=x", RUN_USAGE).0;
    assert_eq!(
        plain,
        format!("unknown investigation flag: --connection=…{RUN_USAGE}")
    );
    let bare = unknown_flag_error("investigation flag", "--bogus", RUN_USAGE).0;
    assert_eq!(
        bare,
        format!("unknown investigation flag: --bogus{RUN_USAGE}")
    );
}
