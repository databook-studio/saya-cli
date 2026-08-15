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
pub(crate) mod discover;
pub(crate) mod io;
mod queue;
mod recall;
mod reconcile;
mod retrieval;
mod review;
mod selection;
pub(crate) mod terms;
mod validity;
mod view;

#[cfg(test)]
mod tests;

// This `pub(crate)` surface is the contract operations API the adapter slices
// (2b-2/3/4: rendering, clap, slash, agent tools) will consume. Nothing in this
// crate references it yet outside tests, so the re-exports read as unused in a
// lib build — they are not dead code, they are the boundary this slice exposes.
#[allow(unused_imports)]
pub(crate) use queue::{QUEUE_DEFAULT_LIMIT, QueuedCandidate, review_queue};
#[allow(unused_imports)]
pub(crate) use recall::{RecallBounds, RecallMode, RecallRequest, recall};
#[allow(unused_imports)]
pub(crate) use reconcile::{ReconcileOutcome, reconcile};
#[allow(unused_imports)]
pub(crate) use retrieval::RetrievalPolicy;
#[allow(unused_imports)]
pub(crate) use review::{ContractOpError, confirm, edit, forget, propose, reject, show};
// 6b import/export: the typed operations the `contracts import`/`export`
// adapter renders. Nothing outside this module references them yet in a
// non-test build, so the re-exports read as unused — they are the boundary this
// slice exposes, like the others above.
#[allow(unused_imports)]
pub(crate) use availability::{
    MODEL_SCHEMA_MAX_AGE_MS, SchemaAvailability, SchemaFreshness, now_unix_ms,
};
#[allow(unused_imports)]
pub(crate) use io::{ExportOutcome, ImportReport, export_contracts, import_contracts};
#[allow(unused_imports)]
pub(crate) use validity::schema_state_for;
#[allow(unused_imports)]
pub(crate) use view::{
    ContractConflict, ContractSchemaState, RecallDiagnostics, RecallOutcome, RetrievedContract,
};
// `PromptTerms` is the prompt-recall signal the agent runtime (2b-3b) consumes
// alongside the recall request types above.
#[allow(unused_imports)]
pub(crate) use terms::PromptTerms;
