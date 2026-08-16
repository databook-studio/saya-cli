//! TUI adapters for the contract slash commands.
//!
//! The TUI runs under the alternate screen, so the shared headless
//! `run_contracts` dispatcher — which `emit`s to a thread-local
//! seam — is wrapped here: the session's active profile is stamped onto the
//! command (the same field the headless `--profile` flag sets, not a second
//! resolution or a second privacy decision), the dispatcher runs, and its
//! captured output is pushed into the transcript as a system or error block.
//!
//! Extracted from `dispatch_actions.rs` to keep that file under the size cap;
//! the seam that routes a command at the active profile lives here, with its
//! tests.

use super::transcript::{BlockKind, Transcript};
use crate::cli::ContractsCommand;
use crate::commands::{
    capture_output_start, capture_output_take, run_contracts as run_contracts_command,
};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_resume::block_on;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;

/// Runs a contract slash command through the shared `run_contracts` dispatcher
/// and pushes its rendered output into the transcript. The TUI runs under the
/// alternate screen, so `run_contracts`'s `emit` output is captured through the
/// thread-local seam instead of going to the process stdout; a non-zero exit
/// surfaces as an error block.
pub(super) fn run_contracts(
    transcript: &mut Transcript,
    state: &SessionState,
    runtime: &RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
    format: RenderFormat,
    command: &ContractsCommand,
) {
    // The session's selected profile maps to the headless `--profile` field:
    // setting it (not a second resolution) routes the command at the database
    // the user /connect-ed to, matching every other TUI slash command.
    let command = with_profile(command, state.profile.as_deref());
    capture_output_start();
    // The shared dispatcher returns Ok(code) for every typed outcome (a store
    // error is emitted as a diagnostic and returned non-zero); the outer Err is
    // a render/IO failure, surfaced here as an error block.
    let code = match block_on(run_contracts_command(command, runtime, format, state_db)) {
        Ok(code) => code,
        Err(error) => {
            capture_output_take();
            transcript.push(BlockKind::Error, error.to_string());
            return;
        }
    };
    let (out, err) = capture_output_take();
    let body = if out.trim().is_empty() { err } else { out };
    if code == 0 {
        transcript.push(BlockKind::System, body.trim_end().to_string());
    } else {
        transcript.push(BlockKind::Error, body.trim_end().to_string());
    }
}

/// Sets the `profile` field on a `ContractsCommand` to the session's active
/// profile. This is the same field the headless `--profile` flag sets; it is
/// not a second resolution or a second privacy decision.
///
/// The match is exhaustive on purpose: every variant is named, so a future
/// profile-bearing variant the slash path routes through the TUI is a compile
/// error here, not a silent fall-through that reads the configured default's
/// data. `Review` and `Forget` have no `profile` field — they address a claim
/// by id — so they pass through unchanged.
fn with_profile(command: &ContractsCommand, profile: Option<&str>) -> ContractsCommand {
    let profile = profile.map(str::to_string);
    match command {
        ContractsCommand::List { .. } => ContractsCommand::List { profile },
        ContractsCommand::Show { table, .. } => ContractsCommand::Show {
            table: table.clone(),
            profile,
        },
        ContractsCommand::Queue { limit, .. } => ContractsCommand::Queue {
            profile,
            limit: *limit,
        },
        ContractsCommand::Remember {
            table,
            kind,
            value,
            column,
            ..
        } => ContractsCommand::Remember {
            table: table.clone(),
            kind: *kind,
            value: value.clone(),
            column: column.clone(),
            profile,
        },
        ContractsCommand::Review {
            claim_id,
            confirm,
            reject,
        } => ContractsCommand::Review {
            claim_id: claim_id.clone(),
            confirm: *confirm,
            reject: *reject,
        },
        // `Decide` (spec D) carries a `profile` field like the other profiled
        // reads/writes: the TUI stamps the session's active profile so a
        // `/confirm c-xxxx` resolves against the database the user /connect-ed
        // to, not the configured default (cross-profile isolation, the same
        // invariant `/queue`'s stamp upholds).
        ContractsCommand::Decide {
            prefix, decision, ..
        } => ContractsCommand::Decide {
            prefix: prefix.clone(),
            decision: *decision,
            profile,
        },
        ContractsCommand::Forget { claim_id, reason } => ContractsCommand::Forget {
            claim_id: claim_id.clone(),
            reason: *reason,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ForgetReasonArg;

    // The active profile a /connect set, differing from any configured default.
    const ACTIVE: &str = "staging";

    /// Every `ContractsCommand` variant that carries a `profile` field must take
    /// the session's active profile when routed through the TUI — the same field
    /// the headless `--profile` flag sets. A variant that falls through loses the
    /// profile and silently reads the configured default, defeating cross-profile
    /// isolation. `/queue` is where a human confirms a claim, so the queue showing
    /// another profile's candidates is the worst case.
    #[test]
    fn with_profile_stamps_active_profile_on_every_profiled_contract_variant() {
        // `/queue` parses to Queue { profile: None, limit }; the TUI must route it
        // at the active profile, not the configured default.
        let queue = ContractsCommand::Queue {
            profile: None,
            limit: Some(20),
        };
        assert_eq!(
            with_profile(&queue, Some(ACTIVE)),
            ContractsCommand::Queue {
                profile: Some(ACTIVE.into()),
                limit: Some(20),
            },
            "/queue lost the active profile — it would read the configured default's candidates"
        );

        // The other profile-bearing read/write variants, for completeness.
        assert_eq!(
            with_profile(&ContractsCommand::List { profile: None }, Some(ACTIVE)),
            ContractsCommand::List {
                profile: Some(ACTIVE.into()),
            }
        );
        assert_eq!(
            with_profile(
                &ContractsCommand::Show {
                    table: "a.b.c".into(),
                    profile: None,
                },
                Some(ACTIVE),
            ),
            ContractsCommand::Show {
                table: "a.b.c".into(),
                profile: Some(ACTIVE.into()),
            }
        );
        assert_eq!(
            with_profile(
                &ContractsCommand::Remember {
                    table: "a.b.c".into(),
                    kind: crate::cli::ClaimKindArg::Alias,
                    value: "v".into(),
                    column: None,
                    profile: None,
                },
                Some(ACTIVE),
            ),
            ContractsCommand::Remember {
                table: "a.b.c".into(),
                kind: crate::cli::ClaimKindArg::Alias,
                value: "v".into(),
                column: None,
                profile: Some(ACTIVE.into()),
            }
        );
    }

    /// Claim-keyed variants (`Review`, `Forget`) have no profile field — they
    /// address a claim by id, so the active profile must NOT be injected. This
    /// guards against an over-broad fix that stamps a profile where none exists.
    #[test]
    fn with_profile_leaves_claim_keyed_variants_untouched() {
        let review = ContractsCommand::Review {
            claim_id: "c-1".into(),
            confirm: true,
            reject: false,
        };
        assert_eq!(with_profile(&review, Some(ACTIVE)), review);

        let forget = ContractsCommand::Forget {
            claim_id: "c-1".into(),
            reason: ForgetReasonArg::UserRequest,
        };
        assert_eq!(with_profile(&forget, Some(ACTIVE)), forget);
    }

    /// `None` active profile (no /connect yet) must not invent one: the command
    /// keeps `profile: None` and the dispatcher resolves the configured default.
    #[test]
    fn with_profile_none_keeps_none() {
        let queue = ContractsCommand::Queue {
            profile: None,
            limit: None,
        };
        assert_eq!(
            with_profile(&queue, None),
            ContractsCommand::Queue {
                profile: None,
                limit: None,
            }
        );
    }
}
