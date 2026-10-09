//! Tests for the prompt-recall context block assembly — slice 2b-3b §5.
//!
//! These assert on the `Vec<ContextBlock>` this layer produces (its length,
//! labels, `truncated` flag, and body content). That a recalled claim never
//! reaches a system message is owned at the runtime boundary — the
//! composition test `recalled_claim_rides_the_user_block_and_never_any_system_message`
//! in `runtime_tests.rs` asserts it against the requests the provider actually
//! receives. The wrapping,
//! escaping, and placement in the user turn are `saya-agent`'s Phase 2a,
//! asserted there; this layer must not pre-escape the body (test 10).
//!
//! The scenarios mirror plan §15 scenario 1 (test 1) and the §5 list.

use super::*;
use crate::contracts::{RecallBounds, RecallMode};
use async_trait::async_trait;
use saya_agent::turn_bytes;
use saya_connectors::DatabaseConnector;
use saya_store::{SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, ConnectionError, Database,
    DatabaseObjectKind, DatabaseObjectRef, DatabaseProfile, KnowledgeSlot, KnowledgeState,
    ProfileIdentity, QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect, Table,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::connection::{ConnectionEntry, ConnectionRegistry};

include!("recall_context_tests/fixtures/context.rs");

include!("recall_context_tests/fixtures/schema.rs");

include!("recall_context_tests/fixtures/claims.rs");

#[path = "recall_context_tests/confirmed_context.rs"]
mod confirmed_context;

// ---------------------------------------------------------------------------
#[path = "recall_context_tests/claim_lifecycle.rs"]
mod claim_lifecycle;

// ---------------------------------------------------------------------------
// Test 3: a candidate (unconfirmed) claim never appears
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
#[path = "recall_context_tests/privacy_selection.rs"]
mod privacy_selection;

#[path = "recall_context_tests/injection_and_stale.rs"]
mod injection_and_stale;

// ---------------------------------------------------------------------------
// Test 5: an explicit @ref selects the object even with no term match
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test 6: a prompt matching nothing → no block (not an empty one)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test 7: an unopenable store → no block, no error
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test 8: the opaque identity appears nowhere in the assembled request
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
#[path = "recall_context_tests/review_claims.rs"]
mod review_claims;

// ---------------------------------------------------------------------------
#[path = "recall_context_tests/empty_context.rs"]
mod empty_context;

// ---------------------------------------------------------------------------
// Test (truncated): recall truncation flags the block
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test (no state_db): recall does not run without a store
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test (system_prompt purity): the body, not the system prompt, carries text
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
include!("recall_context_tests/fixtures/candidate_claims.rs");

#[path = "recall_context_tests/candidate_eligibility.rs"]
mod candidate_eligibility;

// ---------------------------------------------------------------------------
include!("recall_context_tests/fixtures/conflict_schemas.rs");

#[path = "recall_context_tests/conflict_basics.rs"]
mod conflict_basics;

#[path = "recall_context_tests/directive_reasons.rs"]
mod directive_reasons;

#[path = "recall_context_tests/conflict_composition.rs"]
mod conflict_composition;

// ===========================================================================
#[path = "recall_context_tests/claim_markers.rs"]
mod claim_markers;

#[path = "recall_context_tests/context_limits.rs"]
mod context_limits;

// ---------------------------------------------------------------------------
include!("recall_context_tests/fixtures/large_descriptions.rs");

#[path = "recall_context_tests/oversized_claims.rs"]
mod oversized_claims;

#[path = "recall_context_tests/rendered_byte_budget.rs"]
mod rendered_byte_budget;

// ===========================================================================
include!("recall_context_tests/fixtures/claim_pair.rs");

#[path = "recall_context_tests/receipt_drops.rs"]
mod receipt_drops;

#[path = "recall_context_tests/receipt_drop_outcomes.rs"]
mod receipt_drop_outcomes;

#[path = "recall_context_tests/receipt_privacy.rs"]
mod receipt_privacy;
