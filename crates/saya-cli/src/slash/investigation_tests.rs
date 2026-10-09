//! Parser tests for the `/investigation` slash adapter: every subcommand
//! translates into the same `InvestigationCommand` the headless `saya
//! investigation` parser produces, `/investigations` is `list`, and the
//! usage errors carry guidance without swallowing value text.

use super::parse_investigation_command;
use crate::cli::InvestigationCommand;
use crate::slash::parse_slash_command;
use std::path::PathBuf;

fn save(
    name: &str,
    description: Option<&str>,
    sql: Option<&str>,
    file: Option<&str>,
    connection: Option<&str>,
) -> InvestigationCommand {
    InvestigationCommand::Save {
        name: name.into(),
        description: description.map(Into::into),
        sql: sql.map(Into::into),
        file: file.map(PathBuf::from),
        connection: connection.map(Into::into),
        param_specs: Vec::new(),
    }
}

fn parsed(name: &str, tail: &str) -> Result<InvestigationCommand, crate::slash::SlashParseError> {
    parse_investigation_command(name, tail)
}

// --- save ------------------------------------------------------------------

include!("investigation_tests/save.rs");

// --- list ------------------------------------------------------------------

include!("investigation_tests/browse.rs");

// --- show / delete ---------------------------------------------------------

include!("investigation_tests/show_delete.rs");

// --- edit ------------------------------------------------------------------

include!("investigation_tests/edit.rs");

// --- export / import -------------------------------------------------------

include!("investigation_tests/transfer.rs");

// --- run -------------------------------------------------------------------

include!("investigation_tests/run_parameters.rs");

// --- dispatch surface ------------------------------------------------------

include!("investigation_tests/dispatch.rs");
