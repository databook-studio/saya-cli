//! The post-refresh reconciliation report: after an explicit schema refresh
//! succeeds, run the 5d reconciler against the fresh live schema and surface
//! its outcome through the diagnostic channel. Split from
//! [`super::connection_schema`] so the orchestrator stays a thin dispatcher.
//!
//! Reconcile is best-effort here — the refresh itself succeeded, so a
//! reconciliation failure is a diagnostic, not a command failure. The message
//! is counts only: it carries no profile identity and no claim text. A refresh
//! that reconciled nothing (no claims, or all current) stays silent: a no-op
//! diagnostic on every refresh is noise, and a clean refresh has long been a
//! stderr-clean event the render tests pin.

use super::output::emit;
use crate::{
    contracts::{ContractOpError, ReconcileOutcome, reconcile},
    render::{RenderFormat, TerminalEvent},
};
use saya_store::SqliteStateStore;
use saya_types::{ProfileIdentity, SchemaTree};

/// Reconciles `identity`'s claims against the just-fetched `schema` and emits
/// a diagnostic when the pass did something worth reporting. Called only on
/// the explicit refresh path, where a fresh live schema is in hand and a user
/// triggered the work.
pub(super) async fn reconcile_after_refresh(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
    schema: &SchemaTree,
    format: RenderFormat,
) {
    let outcome = reconcile(
        store,
        std::slice::from_ref(identity),
        std::slice::from_ref(&(identity.clone(), schema.clone())),
    )
    .await;
    let Some(message) = outcome_message(outcome) else {
        return;
    };
    emit(TerminalEvent::Diagnostic { message }, format);
}

/// The diagnostic for a reconcile outcome, or `None` when the pass was a no-op
/// (nothing examined, nothing marked, nothing skipped, not truncated). A
/// store-unavailable reconcile is always reported — persistence failed mid-run
/// is not a silent event. The truncation tail is the one signal a user cannot
/// infer from the counts alone.
fn outcome_message(outcome: Result<ReconcileOutcome, ContractOpError>) -> Option<String> {
    match outcome {
        Ok(outcome) if outcome.is_trivial() => None,
        Ok(outcome) => Some(format_outcome(&outcome)),
        Err(_) => {
            Some("Schema refresh reconciled claims but the state store was unavailable.".into())
        }
    }
}

fn format_outcome(outcome: &ReconcileOutcome) -> String {
    let mut msg = format!(
        "Schema refresh examined {} claim(s); {} marked stale, {} skipped (no live schema).",
        outcome.examined, outcome.marked_stale, outcome.skipped_unavailable
    );
    if outcome.truncated {
        msg.push_str(" Pass truncated at the 1000-claim bound.");
    }
    msg
}
