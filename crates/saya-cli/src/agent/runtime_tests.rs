//! Tests for the `KnowledgeSupplied` event.
//!
//! Two layers:
//! - **The pure mapping** [`crate::agent::knowledge_event::knowledge_supplied_event`] — the
//!   three-state decision (Off / Skipped / Ran) and the claim mapping, unit-tested
//!   without a live database (tests 3, 4, 5, 7, 8 and the content of test 1).
//! - **The assembled turn** via [`super::run_prompt_with_inputs`] with an
//!   injected mock provider and an idle registry — proves the emit is before
//!   the provider request (test 2), that exactly one event names the supplied
//!   claims (test 1), and that a store failure still emits and still runs the
//!   turn (test 6). The provider and registry are injected because
//!   `run_prompt_with_sink` builds a live provider and a live (connecting)
//!   registry from config, which a unit test cannot supply; the inner
//!   `run_prompt_with_inputs` is the seam.

use super::super::knowledge_event::knowledge_supplied_event;
use super::super::turn_inputs::TurnInputs;
use super::*;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    KnowledgeOutcome, LearningSkipReason, OverrideFindingDto, ProposedClaimDto, ProviderError,
    SuppliedClaimDto, SuppliedContractDto, ToolCall,
};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, ConnectionError, Database,
    DatabaseObjectKind, DatabaseObjectRef, DatabaseProfile, KnowledgeSlot, KnowledgeState,
    ProfileIdentity, QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect, Table,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config::runtime::RuntimeConfig;
use crate::contracts::{RecallOutcomeKind, RecallReceipt};

#[cfg(test)]
#[path = "runtime_metrics_tests.rs"]
mod metrics_tests;
#[path = "runtime_preflight_tests.rs"]
mod preflight_tests;
#[path = "runtime_timer_tests.rs"]
mod timer_tests;

// ---------------------------------------------------------------------------
// shared harness
// ---------------------------------------------------------------------------

include!("runtime_tests/fixtures/base.rs");
include!("runtime_tests/fixtures/runtime_config.rs");

#[path = "runtime_tests/supplied_event.rs"]
mod supplied_event;

// ===========================================================================
// Test 2: the event is emitted BEFORE any provider request. Asserted against
// the sink's event sequence AND a shared log the provider writes at call time,
// not merely that the event appears (the bug being prevented is emitting
// after the answer).
// ===========================================================================

#[path = "runtime_tests/provider_order.rs"]
mod provider_order;

// ===========================================================================
// Tests 3 & 4: the three-state decision (Off / Skipped / Ran), as pure tests
// on `knowledge_supplied_event`. The runtime calls this; the three states are
// distinguishable here without a live database.
// ===========================================================================

#[path = "runtime_tests/outcome_states.rs"]
mod outcome_states;

#[path = "runtime_tests/event_persistence.rs"]
mod event_persistence;

fn outcome_of(event: &AgentEvent) -> KnowledgeOutcome {
    match event {
        AgentEvent::KnowledgeSupplied { outcome, .. } => *outcome,
        _ => panic!("not a KnowledgeSupplied event"),
    }
}

// ===========================================================================
// Test 5: a candidate claim's status survives into the event, distinct from
// confirmed.
// ===========================================================================

// ===========================================================================
include!("runtime_tests/fixtures/extraction_provider.rs");

#[path = "runtime_tests/extraction_proposed.rs"]
mod extraction_proposed;

#[path = "runtime_tests/extraction_usage.rs"]
mod extraction_usage;

#[path = "runtime_tests/extraction_failures.rs"]
mod extraction_failures;

#[path = "runtime_tests/contract_tools.rs"]
mod contract_tools;

// TA-04: a recalled claim is untrusted data for the user turn — never a
// system message. The adapter-level tests (recall_context_tests.rs) assert
// the block this layer produces; this test runs the real composition and
// asserts on the requests the provider actually received: the planted claim
// sentinel rides the user-side recall block, and no system-role message —
// the turn's assembled prompt or the post-turn extractor's — carries it. The
// long answer admits the post-turn extraction (the gate's object-activity
// trigger registers the supplied contract), so both system-message lanes the
// composition opens are in the recording.
// ===========================================================================
#[path = "runtime_tests/recall_context.rs"]
mod recall_context;

#[path = "runtime_tests/runtime_posture.rs"]
mod runtime_posture;

// ===========================================================================
include!("runtime_tests/fixtures/overrides.rs");

#[path = "runtime_tests/overrides_confirmed.rs"]
mod overrides_confirmed;

#[path = "runtime_tests/overrides_candidates.rs"]
mod overrides_candidates;

include!("runtime_tests/fixtures/learning_breaker.rs");

#[path = "runtime_tests/learning_wait.rs"]
mod learning_wait;

#[path = "runtime_tests/learning_misses.rs"]
mod learning_misses;

#[path = "runtime_tests/learning_policy.rs"]
mod learning_policy;

#[path = "runtime_tests/learning_error.rs"]
mod learning_error;

// ===========================================================================
include!("runtime_tests/fixtures/interactive_tools.rs");

#[path = "runtime_tests/interactive_session.rs"]
mod interactive_session;
