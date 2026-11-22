//! A bare `/investigation save` filled from the query the surface shows.
//!
//! `/investigation save <name>` with neither `--sql` nor `--file` saves the
//! latest successful, concrete query on the connection that actually ran it.
//! An explicit `--sql` or `--file` always wins; the current profile is never
//! substituted silently. The rule lived on the old TUI's dispatcher until X1
//! deleted it: the only caller is this surface's direct-command worker, so the
//! fill moved here with the behaviour intact.

use crate::cli::InvestigationCommand;
use crate::interactive::tui::LastQuery;

/// Fills a save that carries neither `--sql` nor `--file` from the session's
/// last selectable query: the SQL it ran and the connection that ran it.
/// Everything else passes through unchanged.
pub(crate) fn fill_save_from_last_query(
    command: InvestigationCommand,
    last_query: &Option<LastQuery>,
) -> Result<InvestigationCommand, String> {
    let InvestigationCommand::Save {
        name,
        description,
        sql,
        file,
        connection,
        param_specs,
    } = command
    else {
        return Ok(command);
    };
    // An explicit `--sql` / `--file` always wins: the last query is left out
    // entirely, its connection included.
    if sql.is_some() || file.is_some() {
        return Ok(InvestigationCommand::Save {
            name,
            description,
            sql,
            file,
            connection,
            param_specs,
        });
    }
    let Some(last) = last_query else {
        return Err("Nothing to save yet: run a query first (or pass --sql).".into());
    };
    let Some(connection) = connection.or_else(|| last.connection.clone()) else {
        return Err(
            "The last query did not name its connection; re-run it with /sql, or pass \
             --connection."
                .into(),
        );
    };
    Ok(InvestigationCommand::Save {
        name,
        description,
        sql: Some(last.sql.clone()),
        file: None,
        connection: Some(connection),
        param_specs,
    })
}

#[cfg(test)]
#[path = "direct_save_tests.rs"]
mod tests;
