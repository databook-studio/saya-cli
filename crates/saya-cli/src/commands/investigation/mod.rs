//! The headless `saya investigation` adapter (S6/S8): save, list, show,
//! delete, export, and import portable saved-investigation documents through
//! `saya_store::investigations::InvestigationRepository`.
//!
//! One operation, multiple adapters: this is the single dispatcher the clap
//! subcommand uses today and the slash/TUI adapters (S7/S9) will reuse;
//! behavior lives in the per-operation modules and the repository. Saving
//! and importing validate and never execute — no AI provider is constructed
//! anywhere in this module, and nothing here connects to a database.

mod connection;
mod delete;
mod export;
mod fingerprint;
mod import;
mod list;
mod objects;
mod run;
mod run_binding;
mod run_outcome;
mod run_report;
mod save;
mod save_input;
mod show;
#[cfg(test)]
mod tests;

use crate::cli::InvestigationCommand;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
pub use run_outcome::{Replay, RunOutcome};
use saya_store::{InvestigationRepository, SqliteStateStore, StoreError};
use saya_types::investigation::InvestigationId;

/// Exit code for typed investigation-command failures (usage and domain
/// errors); ad-hoc per command like the rest of the crate, matching
/// `contracts`.
pub(super) const EXIT_INVESTIGATION_ERROR: i32 = 2;
/// Exit code when the investigations store cannot be read or opened.
pub(super) const EXIT_STORE_UNAVAILABLE: i32 = 3;
/// Exit code when the safety layer refuses the SQL — the code `query` uses.
pub(super) const EXIT_SAFETY: i32 = 4;

/// Runs one investigation subcommand against the on-disk repository for this
/// invocation (D2 root: the composed runtime's `investigations_root` —
/// resolved once at composition, never re-read from the environment here).
pub async fn run_investigation(
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    run_investigation_in(
        &InvestigationRepository::new(runtime.investigations_root.clone()),
        command,
        runtime,
        format,
        can_prompt,
        state_db,
    )
    .await
}

/// The typed-outcome seam for the replay adapter (D12/C3): the same path as
/// [`run_investigation`], returning the exit code plus the replay (result
/// and evidence) after a successful execution, so a caller captures the run
/// instead of re-parsing rendered output.
pub async fn run_investigation_outcome(
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    run_investigation_outcome_in(
        &InvestigationRepository::new(runtime.investigations_root.clone()),
        command,
        runtime,
        format,
        can_prompt,
        state_db,
    )
    .await
}

/// The repository-explicit seam: tests and later adapters pass the repository
/// they own instead of relying on process env; the public entry is a
/// one-liner over this.
pub(crate) async fn run_investigation_in(
    repo: &InvestigationRepository,
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    run_investigation_outcome_in(repo, command, runtime, format, can_prompt, state_db)
        .await
        .map(|outcome| outcome.code)
}

/// The one dispatcher: every adapter maps what it needs from the outcome —
/// the exit code (headless CLI) or the whole outcome (TUI replay capture).
async fn run_investigation_outcome_in(
    repo: &InvestigationRepository,
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    match command {
        InvestigationCommand::Save {
            name,
            description,
            sql,
            file,
            connection,
        } => save::save(
            repo,
            runtime,
            format,
            save_input::SaveRequest {
                name: &name,
                description: description.as_deref(),
                sql,
                file,
                connection: connection.as_deref(),
            },
        )
        .map(RunOutcome::plain),
        InvestigationCommand::List { limit, offset } => {
            list::list(repo, format, limit, offset).map(RunOutcome::plain)
        }
        InvestigationCommand::Show { id } => show::show(repo, format, &id).map(RunOutcome::plain),
        InvestigationCommand::Delete { id, revision } => {
            delete::delete(repo, format, &id, revision).map(RunOutcome::plain)
        }
        InvestigationCommand::Export {
            id,
            path,
            overwrite,
        } => export::export(repo, format, &id, &path, overwrite).map(RunOutcome::plain),
        InvestigationCommand::Import { path } => {
            import::import(repo, format, &path).map(RunOutcome::plain)
        }
        InvestigationCommand::Run {
            id,
            connection,
            revalidate,
            report,
            rows,
            overwrite,
        } => {
            run::run(
                repo,
                runtime,
                format,
                can_prompt,
                state_db,
                run::RunRequest {
                    id: &id,
                    connection: connection.as_deref(),
                    revalidate,
                    report: report.as_deref(),
                    rows,
                    overwrite,
                },
            )
            .await
        }
    }
}

/// Parses a user-supplied id. A malformed id cannot exist on disk, so it maps
/// to the same "no investigation <id>" refusal the store's `NotFound` gets.
pub(super) fn parse_investigation_id(raw: &str) -> Result<InvestigationId, (i32, String)> {
    InvestigationId::parse(raw)
        .map_err(|_| (EXIT_INVESTIGATION_ERROR, format!("no investigation {raw}")))
}

/// Emits a payload-free store error mapped onto its exit code, naming `id`
/// for the not-found case.
pub(super) fn store_failure(
    error: StoreError,
    id: &str,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (code, message) = store_error_parts(&error, id);
    super::output::failure_message(code, message, format)
}

/// The (exit code, message) mapping for a store error (invariant 5). Store
/// errors are payload-free, so these words are all a store refusal gets.
pub(super) fn store_error_parts(error: &StoreError, id: &str) -> (i32, String) {
    match error {
        StoreError::NotFound => (EXIT_INVESTIGATION_ERROR, format!("no investigation {id}")),
        StoreError::Conflict => (
            EXIT_INVESTIGATION_ERROR,
            "the saved investigation changed underneath this command; retry".to_string(),
        ),
        StoreError::LimitExceeded => (
            EXIT_INVESTIGATION_ERROR,
            "the investigations collection is full (500); delete or export some investigations"
                .to_string(),
        ),
        StoreError::VersionUnsupported => (
            EXIT_INVESTIGATION_ERROR,
            "this investigation was made by a newer saya".to_string(),
        ),
        StoreError::Invalid => (
            EXIT_INVESTIGATION_ERROR,
            "the investigation document is corrupt or invalid".to_string(),
        ),
        StoreError::Unavailable | StoreError::OpenFailed => {
            (EXIT_STORE_UNAVAILABLE, error.to_string())
        }
        // `StoreError` is `#[non_exhaustive]`: a variant added later is a
        // store-level failure the caller cannot classify, so it surfaces with
        // the unavailable exit code and the store's own words — never silently.
        _ => (EXIT_STORE_UNAVAILABLE, error.to_string()),
    }
}
