//! The manual Debug for [`InvestigationCommand`] (F-5): kept beside `cli.rs`
//! so the clap-surface file does not grow. The command is embedded in
//! Debug-implementing carriers — the TUI replay task, the session command
//! enum — so a bound `--param` value must never ride diagnostics with it.
//! Everything else prints as the derive would.

use super::InvestigationCommand;

impl std::fmt::Debug for InvestigationCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Save {
                name,
                description,
                sql,
                file,
                connection,
                param_specs,
            } => formatter
                .debug_struct("Save")
                .field("name", name)
                .field("description", description)
                .field("sql", sql)
                .field("file", file)
                .field("connection", connection)
                .field("param_specs", param_specs)
                .finish(),
            Self::Edit {
                id,
                name,
                description,
                sql,
                file,
                param_specs,
            } => formatter
                .debug_struct("Edit")
                .field("id", id)
                .field("name", name)
                .field("description", description)
                .field("sql", sql)
                .field("file", file)
                .field("param_specs", param_specs)
                .finish(),
            Self::List { limit, offset } => formatter
                .debug_struct("List")
                .field("limit", limit)
                .field("offset", offset)
                .finish(),
            Self::Show { id } => formatter.debug_struct("Show").field("id", id).finish(),
            Self::Delete { id, revision } => formatter
                .debug_struct("Delete")
                .field("id", id)
                .field("revision", revision)
                .finish(),
            Self::Export {
                id,
                path,
                overwrite,
            } => formatter
                .debug_struct("Export")
                .field("id", id)
                .field("path", path)
                .field("overwrite", overwrite)
                .finish(),
            Self::Import { path } => formatter
                .debug_struct("Import")
                .field("path", path)
                .finish(),
            Self::Run {
                id,
                connection,
                revalidate,
                report,
                rows,
                overwrite,
                params,
            } => formatter
                .debug_struct("Run")
                .field("id", id)
                .field("connection", connection)
                .field("revalidate", revalidate)
                .field("report", report)
                .field("rows", rows)
                .field("overwrite", overwrite)
                .field(
                    "params",
                    &params
                        .iter()
                        .map(|binding| redacted_binding(binding))
                        .collect::<Vec<String>>(),
                )
                .finish(),
        }
    }
}

/// One `--param name=value` binding's Debug shape: the name and a marker
/// where the value was — `max_id=1` renders as `max_id=…`. An entry without
/// a name (a malformed binding, refused at bind time) is fully redacted.
fn redacted_binding(binding: &str) -> String {
    match binding.split_once('=') {
        Some((name, _)) => format!("{name}=…"),
        None => "…".to_owned(),
    }
}
