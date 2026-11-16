//! Headless `saya contracts` commands: drives the command adapter directly
//! (constructs `ContractsCommand` values and calls `run_contracts`), never
//! shelling out to a built binary.
//!
//! Output is captured through the thread-local seam in `output::emit` so the
//! tests can assert on rendered text/JSON/NDJSON without touching the process
//! stdout (which would race under parallel test runs).

use saya_cli::{
    ClaimKindArg, Cli, Command, ContractsCommand, ForgetReasonArg, RenderFormat, ReviewDecisionArg,
    RuntimeConfig, capture_output_start, capture_output_take, load_with_sources, profile_identity,
    run_contracts,
};
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, Column, ColumnRequirement, Database, DatabaseObjectKind,
    DatabaseObjectRef, FINGERPRINT_VERSION, KnowledgeSlot, KnowledgeState, ProfileIdentity, Schema,
    SchemaBinding, SchemaFingerprint, SchemaTree, Table,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

include!("contracts_cli/harness.rs");
include!("contracts_cli/claim_lifecycle.rs");
include!("contracts_cli/forget_lifecycle.rs");
include!("contracts_cli/decision_states.rs");
include!("contracts_cli/command_errors.rs");
include!("contracts_cli/output_formats.rs");
include!("contracts_cli/candidate_queue.rs");
include!("contracts_cli/queue_identity.rs");
include!("contracts_cli/schema_cache.rs");
include!("contracts_cli/schema_remember_a.rs");
include!("contracts_cli/schema_remember_b.rs");
include!("contracts_cli/queue_freshness.rs");
include!("contracts_cli/claim_id_prefix.rs");
include!("contracts_cli/profile_selection.rs");
include!("contracts_cli/ambiguous_prefix.rs");
include!("contracts_cli/grammar.rs");
