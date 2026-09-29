//! The `/investigation` slash dispatcher: translates a slash command name plus
//! its verbatim tail into the same `InvestigationCommand` the headless `saya
//! investigation` parser produces, so the slash, headless-REPL, and clap paths
//! hand one value to the shared `run_investigation` operation — no second
//! parsing, store, or rendering lives here. The flat-tail flag grammar it
//! scans with lives in [`flags`].

use crate::cli::InvestigationCommand;
use crate::slash::SlashParseError;
use flags::{id_from, scan, take_number, take_value, take_values, unknown_flag_error};
use std::path::PathBuf;

mod flags;

/// Translates `/investigation …` (or the `/investigations` alias, which is
/// `list`) into the matching `InvestigationCommand`, or a usage error — never
/// "not a slash command".
pub(crate) fn parse_investigation_command(
    name: &str,
    tail: &str,
) -> Result<InvestigationCommand, SlashParseError> {
    const USAGE: &str = " (usage: /investigation <save|list|show|edit|delete|export|import|run> …)";
    if name == "investigations" {
        // Bare /investigations opens the TUI's picker before this parser
        // runs (the TUI dispatch intercepts it); `--list` is the explicit
        // escape hatch back to the text listing.
        if tail.trim() == "--list" {
            return Ok(InvestigationCommand::List {
                limit: None,
                offset: None,
            });
        }
        return parse_list(tail);
    }
    let Some(&(start, end)) = flags::token_spans(tail).first() else {
        return Err(SlashParseError(format!(
            "investigation needs a subcommand{USAGE}"
        )));
    };
    let rest = tail[end..].trim_start();
    match &tail[start..end] {
        "save" => parse_save(rest),
        "list" => parse_list(rest),
        "show" => parse_show(rest),
        "edit" => parse_edit(rest),
        "delete" => parse_delete(rest),
        "export" => parse_export(rest),
        "import" => parse_import(rest),
        "run" => parse_run(rest),
        other => Err(unknown_flag_error("investigation subcommand", other, USAGE)),
    }
}

fn parse_save(tail: &str) -> Result<InvestigationCommand, SlashParseError> {
    const USAGE: &str = " (usage: /investigation save <name> [--description <words>] \
                         [--sql <SQL>] [--file <PATH>] [--param-spec <SPEC>] [--connection <PROFILE>])";
    let mut scan = scan(
        tail,
        &["--description", "--sql", "--file", "--connection"],
        &[],
        &["--param-spec"],
        USAGE,
    )?;
    let Some(name) = Some(scan.positional).filter(|name| !name.is_empty()) else {
        return Err(SlashParseError(format!(
            "investigation save needs a name{USAGE}"
        )));
    };
    Ok(InvestigationCommand::Save {
        name,
        description: take_value(&mut scan.values, "--description"),
        sql: take_value(&mut scan.values, "--sql"),
        file: take_value(&mut scan.values, "--file").map(PathBuf::from),
        connection: take_value(&mut scan.values, "--connection"),
        param_specs: take_values(&mut scan.repeats, "--param-spec"),
    })
}

fn parse_list(tail: &str) -> Result<InvestigationCommand, SlashParseError> {
    const USAGE: &str = " (usage: /investigation list [--limit N] [--offset N])";
    let mut scan = scan(tail, &["--limit", "--offset"], &[], &[], USAGE)?;
    if !scan.positional.is_empty() {
        return Err(SlashParseError(format!("unexpected argument{USAGE}")));
    }
    Ok(InvestigationCommand::List {
        limit: take_number(&mut scan.values, "--limit", USAGE)?,
        offset: take_number(&mut scan.values, "--offset", USAGE)?,
    })
}

fn parse_show(tail: &str) -> Result<InvestigationCommand, SlashParseError> {
    const USAGE: &str = " (usage: /investigation show <id>)";
    let scan = scan(tail, &[], &[], &[], USAGE)?;
    Ok(InvestigationCommand::Show {
        id: id_from(&scan.positional, USAGE)?,
    })
}

/// `/investigation edit <id> [--name <words>] [--description <words>]
/// [--sql <SQL>] [--file <PATH>] [--param-spec <SPEC>]`: the same `Edit`
/// command the clap parser builds, with the id positional and at least one
/// edit flag required by the operation itself.
fn parse_edit(tail: &str) -> Result<InvestigationCommand, SlashParseError> {
    const USAGE: &str = " (usage: /investigation edit <id> [--name <words>] \
                         [--description <words>] [--sql <SQL>] [--file <PATH>] [--param-spec <SPEC>])";
    let mut scan = scan(
        tail,
        &["--name", "--description", "--sql", "--file"],
        &[],
        &["--param-spec"],
        USAGE,
    )?;
    Ok(InvestigationCommand::Edit {
        id: id_from(&scan.positional, USAGE)?,
        name: take_value(&mut scan.values, "--name"),
        description: take_value(&mut scan.values, "--description"),
        sql: take_value(&mut scan.values, "--sql"),
        file: take_value(&mut scan.values, "--file").map(PathBuf::from),
        param_specs: take_values(&mut scan.repeats, "--param-spec"),
    })
}

fn parse_delete(tail: &str) -> Result<InvestigationCommand, SlashParseError> {
    const USAGE: &str = " (usage: /investigation delete <id> [--revision N])";
    let mut scan = scan(tail, &["--revision"], &[], &[], USAGE)?;
    Ok(InvestigationCommand::Delete {
        id: id_from(&scan.positional, USAGE)?,
        revision: take_number(&mut scan.values, "--revision", USAGE)?,
    })
}

fn parse_export(tail: &str) -> Result<InvestigationCommand, SlashParseError> {
    const USAGE: &str = " (usage: /investigation export <id> [--overwrite] <path>)";
    let spans = flags::token_spans(tail);
    // The id is the first token (ids never contain whitespace); the path is
    // the rest of the tail after the flags, verbatim, spaces included.
    let id = match spans.first() {
        Some(&(start, end)) if !tail[start..end].starts_with("--") => tail[start..end].to_string(),
        _ => return Err(SlashParseError(format!("expected a single id{USAGE}"))),
    };
    let mut overwrite = false;
    for &(start, end) in spans.iter().skip(1) {
        let token = &tail[start..end];
        if token == "--overwrite" && overwrite {
            return Err(SlashParseError(format!("--overwrite given twice{USAGE}")));
        }
        if token == "--overwrite" {
            overwrite = true;
        } else if token.starts_with("--") {
            return Err(unknown_flag_error("investigation flag", token, USAGE));
        } else {
            return Ok(InvestigationCommand::Export {
                id,
                path: PathBuf::from(tail[start..].trim()),
                overwrite,
            });
        }
    }
    Err(SlashParseError(format!(
        "export needs a destination path{USAGE}"
    )))
}

fn parse_import(tail: &str) -> Result<InvestigationCommand, SlashParseError> {
    const USAGE: &str = " (usage: /investigation import <path>)";
    // The path is the whole tail verbatim; import takes no flags.
    let scan = scan(tail, &[], &[], &[], USAGE)?;
    if scan.positional.is_empty() {
        return Err(SlashParseError(format!("import needs a path{USAGE}")));
    }
    Ok(InvestigationCommand::Import {
        path: PathBuf::from(&scan.positional),
    })
}

/// `/investigation run <id> [--connection <PROFILE>] [--revalidate]
/// [--param <NAME=VALUE> …]`: the same `Run` command the clap parser builds;
/// `--param` is repeatable and rides the background replay through.
fn parse_run(tail: &str) -> Result<InvestigationCommand, SlashParseError> {
    const USAGE: &str = " (usage: /investigation run <id> [--connection <PROFILE>] \
                         [--revalidate] [--param <NAME=VALUE> …])";
    let mut scan = scan(
        tail,
        &["--connection"],
        &["--revalidate"],
        &["--param"],
        USAGE,
    )?;
    Ok(InvestigationCommand::Run {
        id: id_from(&scan.positional, USAGE)?,
        connection: take_value(&mut scan.values, "--connection"),
        revalidate: scan.booleans.contains(&"--revalidate"),
        params: take_values(&mut scan.repeats, "--param"),
        // The TUI writes reports with /report; the slash run takes no report flags.
        report: None,
        rows: None,
        overwrite: false,
    })
}

#[cfg(test)]
#[path = "investigation_tests.rs"]
mod tests;
