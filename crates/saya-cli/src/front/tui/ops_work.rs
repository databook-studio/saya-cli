//! The reads and writes a surface command states that are not the session's own
//! state: the panel, the schema, a saved session, a preference, a contract, a
//! recovery record, the configuration check, the permission view, a grant
//! statement, and the two query-shaped reads.
//!
//! Every arm calls the operation the line front's loop calls, with the same
//! arguments, and lands what it printed as one transcript entry — a notice when
//! it succeeded, an error when it did not. The printed-output operations are
//! captured through the crate's one seam ([`crate::commands::capture_output`]),
//! the same one the surface's `/investigation` already uses, so a shared
//! operation needs no second rendering to reach this surface.

use super::ops::{Asked, OpsDeps};
use super::ops_work_io::{
    EXPLAIN_NEEDS_A_STATEMENT, block_on, captured, section_answer, statement,
};
use crate::chat::ops_status::Section;

/// One shared operation the port performs off the loop's thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Work {
    /// `/status`, and the two aliases that open one of its sections.
    Status(Option<Section>),
    /// `/schema [refresh]` — the schema view, or a refresh of it.
    Schema { refresh: bool },
    /// `/explain [sql]` — the statement's plan.
    Explain(String),
    /// `/sessions`, `/sessions list` — the shared saved-session listing.
    SessionsList,
    /// `/sessions delete <id>` — the shared delete, never prompting.
    SessionsDelete(String),
    /// `/preferences …` — the worktree's project preferences.
    Preferences(crate::cli::PreferencesCommand),
    /// `/contracts …` — the knowledge store's one operation.
    Contracts(crate::slash::ContractsRequest),
    /// `/recovery …` — the session's per-edit recovery store.
    Recovery(crate::commands::recovery::RecoveryCommand),
    /// `/doctor` — the configuration check.
    Doctor,
    /// `/permissions` — one read-only view of what this session may do.
    Permissions(crate::slash::PermissionsRequest),
    /// `/allow <scopes…>` — seed the session's grant store.
    Allow(Vec<String>),
    /// `/sql <query>` — one bounded read-only statement.
    Sql(String),
    /// `/investigation …` — saved investigations.
    Investigation(crate::cli::InvestigationCommand),
}

/// Performs one shared operation and answers with the words it produced.
pub(crate) fn perform(deps: &OpsDeps, work: Work) -> Asked {
    match work {
        Work::Status(None) => {
            let runtime = deps.runtime.current();
            let state = deps.state();
            Asked::Notice(crate::chat::ops_status::panel_text(
                &state,
                &runtime,
                &deps.store,
                deps.root.as_deref(),
            ))
        }
        Work::Status(Some(section)) => Asked::Notice(section_answer(deps, section)),
        Work::Doctor => Asked::Notice(crate::config::doctor::summary(&deps.runtime.current())),
        Work::SessionsList => {
            match crate::interactive::session::commands::sessions_list_text(
                &deps.store,
                &crate::interactive::session::lifecycle::paths::default_session_dir(),
            ) {
                Ok(text) => Asked::Notice(text),
                Err(text) => Asked::Error(text),
            }
        }
        Work::SessionsDelete(id) => {
            let id = id.as_str();
            let my_id = deps.state().id.clone();
            let (ok, text) = crate::interactive::session::commands::sessions_delete_text(
                &deps.store,
                &crate::interactive::session::lifecycle::paths::default_session_dir(),
                &crate::commands::coding::job_records::root(),
                id,
                Some(&my_id),
            );
            if ok {
                Asked::Notice(text)
            } else {
                Asked::Error(text)
            }
        }
        Work::Preferences(command) => {
            let root = deps.root.clone();
            let (code, text) = block_on(crate::commands::preferences::run_in_session(
                command,
                root.as_deref(),
                &deps.state_db,
            ));
            if code == 0 {
                Asked::Notice(text)
            } else {
                Asked::Error(text)
            }
        }
        Work::Contracts(request) => {
            let runtime = deps.runtime.current();
            captured(
                crate::commands::run_contracts_request(
                    request,
                    &runtime,
                    deps.format,
                    &deps.state_db,
                ),
                |code| code == 0,
            )
        }
        Work::Recovery(command) => {
            let universe = deps.universe.workspace();
            let now = crate::interactive::session::continuation::now_unix_ms();
            let (ok, text) = crate::commands::recovery::run_recovery(
                command,
                &deps.recovery,
                universe.as_deref(),
                now,
            );
            if ok {
                Asked::Notice(text)
            } else {
                Asked::Error(text)
            }
        }
        Work::Permissions(request) => {
            let runtime = deps.runtime.current();
            let denied = deps.universe.approval_facts(&runtime).denied_programs;
            let mut state = deps.state();
            let text = crate::chat::ops_permissions::answer(
                &mut state,
                request,
                deps.policy.mode(),
                deps.policy.grants(),
                &denied,
            );
            Asked::Notice(text)
        }
        Work::Allow(tokens) => {
            let runtime = deps.runtime.current();
            let facts = deps.universe.approval_facts(&runtime);
            match crate::interactive::session::permissions::grants::allow(
                &tokens,
                &facts,
                deps.policy.grants(),
                &deps.journal,
            ) {
                Ok(text) => Asked::Notice(text),
                Err(text) => Asked::Error(text),
            }
        }
        Work::Schema { refresh } => {
            let runtime = deps.runtime.current();
            let profile = deps.state().profile.clone();
            captured(
                crate::interactive::session_schema::run(
                    &runtime,
                    profile.as_deref(),
                    refresh,
                    false,
                    deps.format,
                    &deps.state_db,
                ),
                |completed| completed,
            )
        }
        Work::Sql(sql) => statement(deps, sql),
        Work::Explain(sql) => {
            if sql.trim().is_empty() {
                return Asked::Notice(EXPLAIN_NEEDS_A_STATEMENT.to_owned());
            }
            statement(deps, format!("EXPLAIN {sql}"))
        }
        Work::Investigation(command) => {
            let runtime = deps.runtime.current();
            captured(
                crate::commands::run_investigation(
                    command,
                    &runtime,
                    deps.format,
                    false,
                    &deps.state_db,
                ),
                |code| code == 0,
            )
        }
    }
}
