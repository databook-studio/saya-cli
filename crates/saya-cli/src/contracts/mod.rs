//! Shared contract-application operations: the typed layer every adapter renders.
//!
//! This slice computes and selects; it writes nothing to the store on the recall
//! path and adds no policy of its own on the review path beyond what
//! [`ContractStore`] already enforces. Presentation (rendering, clap, slash, agent
//! tools) is a later slice — these modules return typed data only.

pub(crate) mod args;
mod assemble;
mod availability;
mod conflict;
mod decide;
mod knowledge_validity;
mod name_match;
mod op_error;
mod queue;
mod recall;
mod receipt;
mod reconcile;
mod retrieval;
mod review;
mod selection;
pub(crate) mod terms;
mod use_once;
mod validity;
mod view;
// P2b-1: the override detector. A pure function nobody calls yet — P2b-2 will
// run it against the SQL the model generated and the [`RecallReceipt`] recall
// supplied. Mirrors the `PromptTerms` re-export pattern below.
mod override_det;

#[cfg(test)]
mod tests;

// This `pub(crate)` surface is the contract operations API the adapter slices
// (2b-2/3/4: rendering, clap, slash, agent tools) will consume. Nothing in this
// crate references it yet outside tests, so the re-exports read as unused in a
// lib build — they are not dead code, they are the boundary this slice exposes.
#[allow(unused_imports)]
pub(crate) use availability::{
    MODEL_SCHEMA_MAX_AGE_MS, SchemaAvailability, SchemaFreshness, now_unix_ms,
};
#[allow(unused_imports)]
pub(crate) use decide::resolve_prefix;
#[allow(unused_imports)]
pub(crate) use op_error::ContractOpError;
#[allow(unused_imports)]
pub(crate) use queue::{QUEUE_DEFAULT_LIMIT, QueuedCandidate, review_queue};
#[allow(unused_imports)]
pub(crate) use recall::{RecallBounds, RecallMode, RecallRequest, recall};
#[allow(unused_imports)]
pub(crate) use reconcile::{ReconcileOutcome, reconcile};
#[allow(unused_imports)]
pub(crate) use retrieval::RetrievalPolicy;
pub(crate) use review::{confirm, forget, propose, reject, show};
#[allow(unused_imports)]
pub(crate) use use_once::use_candidate_once;
#[allow(unused_imports)]
pub(crate) use validity::schema_state_for;
// D-2: the computed validity vocabulary over KnowledgeState. Pure, uncalled
// this slice — the adopting slice will switch to it. Re-exported here
// alongside the other contract operations, reading unused like the others
// until something consumes it.
#[allow(unused_imports)]
pub(crate) use knowledge_validity::{
    KnowledgeValidity, knowledge_state_from_status, knowledge_validity_for,
};
#[allow(unused_imports)]
pub(crate) use view::{
    ContractConflict, ContractSchemaState, RecallDiagnostics, RecallOutcome, RetrievedContract,
};
// P1a: the typed recall receipt. The agent layer (`recall_context`) builds it
// beside the context blocks; nothing consumes it yet (P1b). Re-exported here
// alongside the other contract operations the adapter slices will consume.
#[allow(unused_imports)]
pub(crate) use receipt::{RecallOutcomeKind, RecallReceipt, SuppliedClaim, SuppliedContract};
// `PromptTerms` is the prompt-recall signal the agent runtime (2b-3b) consumes
// alongside the recall request types above.
#[allow(unused_imports)]
pub(crate) use terms::PromptTerms;
// P2b-1: the override detector — pure, uncalled this slice. P2b-2 runs it
// against generated SQL; until then the re-export reads as unused, like the
// others above.
#[allow(unused_imports)]
pub(crate) use override_det::{OverrideFinding, detect_overrides};
