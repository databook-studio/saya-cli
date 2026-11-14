use super::*;
use std::process::Command;

/// `saya open` was removed with the CSV/Parquet file staging. The word is an
/// unknown subcommand: clap refuses it as a usage error (exit `2`) before any
/// file is read and before any runtime loads, so a script passing it never
/// half-runs anything.
#[test]
fn open_is_an_unknown_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["open", "x.csv"])
        .env(
            "SAYA_CONFIG_HOME",
            std::env::temp_dir().join("saya-open-gone"),
        )
        .output()
        .expect("the binary runs");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(
        output.status.code(),
        Some(2),
        "`saya open x.csv` must be a usage error: {stderr}"
    );
    assert!(
        stderr.contains("unrecognized subcommand"),
        "the refusal names the unknown subcommand: {stderr}"
    );
}

/// `saya --help` does not advertise a subcommand that no longer exists: the
/// help surface and the grammar agree.
#[test]
fn help_does_not_list_open() {
    let help = Cli::command().render_help().to_string();
    assert!(
        !help
            .lines()
            .any(|line| line.trim_start().starts_with("open ")),
        "`saya --help` must not list an `open` subcommand: {help}"
    );
}
