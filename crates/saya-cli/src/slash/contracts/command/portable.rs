//! The portable context arm: `/contracts export|import|import-dbt`.

use crate::cli::ContractsCommand;
use crate::slash::SlashParseError;

/// Payload-free usage for the portable context operations (B2c). The path is
/// one token; the flags are fixed forms. Never echoes the untrusted tail on an
/// error.
fn usage_portable(operation: &str) -> String {
    match operation {
        "export" => "/contracts export <path> [--overwrite]".into(),
        "import" => "/contracts import <path> [--preview]".into(),
        _ => "/contracts import-dbt <manifest> [--select <glob>]… [--preview]".into(),
    }
}

/// Parses `/contracts export|import|import-dbt …` into the same
/// `ContractsCommand` the headless parser produces (B2c invariant 5): the
/// path is one token, and the known flags — `--overwrite`, `--preview`, and
/// repeatable `--select <glob>` — are the only ones. The slash path always
/// uses the active profile, matching the other `/contracts` arms.
pub(super) fn parse_portable(
    operation: &str,
    arg: &str,
) -> Result<ContractsCommand, SlashParseError> {
    let usage = || SlashParseError(usage_portable(operation));
    let mut tokens = arg.split_whitespace();
    let path = tokens.next().ok_or_else(usage)?.to_string();
    // A path that starts with `--` is a missing path followed by a flag, the
    // same refusal the headless parser gives; it is never a filename.
    if path.starts_with("--") {
        return Err(usage());
    }
    let mut overwrite = false;
    let mut preview = false;
    let mut select = Vec::new();
    while let Some(token) = tokens.next() {
        match token {
            "--overwrite" if operation == "export" && !overwrite => overwrite = true,
            "--preview" if operation != "export" && !preview => preview = true,
            "--select" if operation == "import-dbt" => {
                select.push(tokens.next().ok_or_else(usage)?.to_string());
            }
            _ => return Err(usage()),
        }
    }
    Ok(match operation {
        "export" => ContractsCommand::Export {
            path: path.into(),
            profile: None,
            overwrite,
        },
        "import" => ContractsCommand::Import {
            path: path.into(),
            profile: None,
            preview,
        },
        _ => ContractsCommand::ImportDbt {
            manifest: path.into(),
            profile: None,
            select,
            preview,
        },
    })
}
