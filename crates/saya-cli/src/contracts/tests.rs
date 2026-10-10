//! Integration tests for the contract-application operations layer.
//!
//! These exercise the `pub(crate)` surface that every adapter will render:
//! recall, validity, conflict detection, and the review wrappers. They are
//! in-crate (not under `tests/`) because the surface is `pub(crate)` — the
//! operations layer returns typed data for saya-cli's own adapters, not for
//! external consumers.

use super::{
    ContractOpError, ContractSchemaState, RecallBounds, RecallDiagnostics, RecallMode,
    RecallRequest, RememberOutcome, RetrievalPolicy, SchemaAvailability, approve_all, confirm,
    conflicts_for, forget, recall, reject, remember, resolve_prefix, show, use_candidate_once,
};
use saya_store::{ForgetReason, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, Database, DatabaseObjectKind,
    DatabaseObjectRef, KnowledgeSlot, KnowledgeState, ProfileIdentity, Schema, SchemaTree, Table,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::contracts::{ContractConflict, RecallOutcome};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

include!("tests/fixtures.rs");
include!("tests/recall_request.rs");
// ---------------------------------------------------------------------------
// Test 1: cross-profile isolation
// ---------------------------------------------------------------------------
include!("tests/profile_recall.rs");

// ---------------------------------------------------------------------------
// Test 3: bounds hold on objects and claims.
//
// The byte bound used to be asserted here too, against serialized claim payloads
// inside `recall`/`assemble`. That assertion encoded the bug the P1 fix removes:
// payload length is not the unit that reaches the request — the rendered block
// (headers, markers, conflict lines, the wrapper) is — and the bound skipped the
// first claim of every object, so it was never a bound at all. The byte bound now
// lives in the prompt-recall path, against the rendered body and the agent message
// budget; its tests are in `recall_context_tests.rs` (the layer that renders). The
// count bounds this test still pins — `max_objects` and `max_claims_per_object` —
// are the ones `recall`/`assemble` actually enforce.
// ---------------------------------------------------------------------------
include!("tests/bounds_and_ambiguity.rs");

// ---------------------------------------------------------------------------
// Test 5: privacy gate returns zero contracts and counts the excluded
// ---------------------------------------------------------------------------
include!("tests/recall_conflicts.rs");

// ---------------------------------------------------------------------------
// Test 9: diagnostics carry no claim text
// ---------------------------------------------------------------------------
include!("tests/diagnostics.rs");

// ---------------------------------------------------------------------------
// Phase 3d: the candidate review queue
// ---------------------------------------------------------------------------
//
// `review_queue` is the opposite view from recall: recall answers "what is
// true about this question", the queue answers "what is waiting for me". It
// lists candidates only, ordered most-evidence-first then oldest then by claim
// id, so a reviewer works a stable list.

use super::review_queue;

include!("tests/queue_listing.rs");

include!("tests/queue_bounds_and_schema.rs");

// ---------------------------------------------------------------------------
// Spec D / Chunk 3: `resolve_prefix` matches `ki-…` knowledge-item ids. The
// receipt/list/show stanzas print `ki-` ids, so a resolver that only matched
// the legacy `c-` ids would silently stop resolving anything a user can see —
// the defect this chunk closes.
// ---------------------------------------------------------------------------
include!("tests/prefix_resolution.rs");

// ---------------------------------------------------------------------------
// Phase 5b-fix: the shared retrieval policy — computed-stale and the human path.
// ---------------------------------------------------------------------------
//
// `show` and `recall` route their output through one policy (`retrieval`). A
// contract computed `Stale` is dropped for the model and counted; kept for a
// human reviewer with its state and reason. `NeedsReview` is never excluded —
// only `Stale` is, and the two states must keep meaning different things.

include!("tests/stale_retrieval.rs");
// P1 / freshness: model vs human path against a stale-by-age cached schema.
//
// A cached schema older than the model bound (24h) must not classify a claim
// `Current` on the model path — "current" must mean "matches the schema now",
// not "matches whatever we last wrote down". It reads `LiveSchemaUnavailable`,
// so the contract is *kept* and labelled (not silently dropped as `Stale`, and
// not silently read as `Current`). The human-review path (`contracts list`/
// `show`/`queue`) is unbounded: the same stale-by-age cache classifies
// normally, because a reviewer is not asked to trust a query built on their
// own contracts. These exercise the policy branch in `recall` directly; the
// pure age comparison is unit-tested in `availability`.
// ---------------------------------------------------------------------------

include!("tests/freshness_paths.rs");
include!("tests/freshness_and_cached_confirm.rs");
// --- Absence is not evidence of absence. An empty cached schema (the no-op
// --- sentinel a fresh store writes — `databases` empty) carries no real schema
// --- information, so it is "no usable schema", not "the table is gone". This
// --- is the same class as review item #29: the old `confirm` ran `find_table`
// --- against the empty tree, got `None`, and returned `ObjectGone` — reading an
// --- empty cache as proof the object no longer exists. `store_at` caches
// --- exactly this empty default, so this is the state `/confirm` lands in
// --- before any `connection schema --refresh`.
include!("tests/confirm_schema_state.rs");

// ---------------------------------------------------------------------------
// Term matching must bridge an ordinary English plural to the singular
// object it names. A user asks "how many rentals…" for the table `rental`;
// `rentals` is longer than `rental` so it can never be a substring of the
// qualified name, and a confirmed claim about that table never reached the
// model. These exercise selection's tier-3 match through the full `recall`
// path — `best_tier` is private to `selection.rs`, so recall is the oracle.
// ---------------------------------------------------------------------------

include!("tests/prompt_term_basics.rs");

include!("tests/prompt_object_selection.rs");

// ---------------------------------------------------------------------------
// Spec C — use_candidate_once: admit one candidate to recall for one turn,
// without confirming it. The admission is in-memory and request-scoped (the
// §4 decision): nothing is persisted, so a claim used once and left cannot be
// found admissible later. These tests exercise the shared operation and the
// per-claim exception in selection; no adapter calls it yet.
// ---------------------------------------------------------------------------

include!("tests/candidate_admission_support.rs");
include!("tests/candidate_scope.rs");

include!("tests/candidate_refusals_and_evidence.rs");

include!("tests/remember_replacement.rs");

// ---------------------------------------------------------------------------
// approve the whole queue, and report every item it refused.
//
// `approve_all` is the batch entry beside `confirm`: it takes the ids of the
// bounded queue the user was shown and runs each one through `confirm()`
// unchanged. A mixed batch is the expected shape — a `Dismissed` item is
// refused as `Conflict` (an item dismissed between the queue read and the
// sweep, e.g. by another session, must be reported, not skipped) — and the
// successes must persist despite the refusals: a partial batch is a success,
// never a rollback.
// ---------------------------------------------------------------------------

include!("tests/approve_batch.rs");

// ---------------------------------------------------------------------------
// The two new slots: a join rule and a metric definition survive store,
// recall and render, and the binding invalidates a metric when one of its
// underlying columns disappears.
// ---------------------------------------------------------------------------

include!("tests/join_rule_recall.rs");

include!("tests/metric_invalidation.rs");
