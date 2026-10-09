//! The statement allowlist: the shapes scratch accepts — reads, the write
//! capability on the scratch file itself, introspection — and the specific
//! label each refused family reports under.

use sqlparser::ast::Statement;

use super::types::ScratchRejection;

/// The allowlist of statement shapes scratch accepts — reads, the write
/// capability (DDL/DML on the scratch file itself), and introspection;
/// everything else is refused by kind (`ATTACH`, `COPY … FROM`/`TO`,
/// `INSTALL`, `LOAD`, `PRAGMA`, `SET`, `CALL`, the file/configuration family).
/// Returns whether the accepted shape hands rows back.
pub(super) fn allowed(statement: &Statement) -> Result<bool, ScratchRejection> {
    match statement {
        Statement::Query(_) => Ok(true),
        Statement::Explain { statement, .. } => match statement.as_ref() {
            Statement::Query(_) => Ok(true),
            other => Err(ScratchRejection::Statement(kind(other))),
        },
        Statement::ShowVariable { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowCollation { .. } => Ok(true),
        Statement::Insert(insert) => Ok(insert.returning.is_some()),
        Statement::CreateTable { .. }
        | Statement::CreateView { .. }
        | Statement::CreateSchema { .. }
        | Statement::CreateIndex { .. }
        | Statement::Update { .. }
        | Statement::Delete { .. }
        | Statement::Drop { .. }
        | Statement::AlterTable { .. }
        | Statement::Truncate { .. }
        | Statement::Merge { .. }
        | Statement::Analyze { .. } => Ok(false),
        other => Err(ScratchRejection::Statement(kind(other))),
    }
}

/// A specific label for the refused statement family — `INSTALL httpfs`,
/// `ATTACH`, `COPY … TO` — so the refusal names what was wrong.
fn kind(statement: &Statement) -> String {
    match statement {
        Statement::Install { extension_name } => format!("INSTALL {}", extension_name.value),
        Statement::Load { extension_name } => format!("LOAD {}", extension_name.value),
        Statement::AttachDatabase { .. } | Statement::AttachDuckDBDatabase { .. } => {
            "ATTACH".to_string()
        }
        Statement::DetachDuckDBDatabase { .. } => "DETACH".to_string(),
        Statement::Copy { to: true, .. } => "COPY … TO".to_string(),
        Statement::Copy { .. } => "COPY … FROM".to_string(),
        Statement::SetVariable { .. }
        | Statement::SetTimeZone { .. }
        | Statement::SetRole { .. }
        | Statement::SetTransaction { .. }
        | Statement::SetNames { .. } => "SET".to_string(),
        Statement::Pragma { .. } => "PRAGMA".to_string(),
        Statement::Call(_) => "CALL".to_string(),
        Statement::Directory { .. } | Statement::LoadData { .. } => "file load".to_string(),
        _ => "this statement".to_string(),
    }
}
