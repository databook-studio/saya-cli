//! Portable reviewed business context (B2c, ADR 0006): `saya contracts
//! export|import|import-dbt` and the `/contracts export|import|import-dbt`
//! slash forms.
//!
//! The export must carry only Active claims with identity stripped; the import
//! must validate the whole document before any store access, map logical
//! objects onto the chosen profile's schema only, land every written item as
//! Pending with no imported authority, treat conflicts and unresolvable
//! objects as reports rather than writes, and be idempotent and atomic. All
//! three operations go through the same `ContractsCommand` the slash adapter
//! translates, so the parity claim is checked by parsing, not by trusting the
//! dispatcher.
//!
//! Output is captured through the thread-local seam in `output::emit`, exactly
//! as `tests/contracts_cli.rs` does.

use saya_cli::{
    ClaimKindArg, ContractsCommand, RenderFormat, RuntimeConfig, SlashCommand,
    capture_output_start, capture_output_take, load_with_sources, parse_slash_command,
    profile_identity, run_contracts,
};
use saya_config::MemoryMode;
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    CONTEXT_FORMAT, CONTEXT_FORMAT_VERSION, ClaimOrigin, ClaimPayload, Column, ContextDocumentV1,
    ContextItem, Database, DatabaseObjectKind, DatabaseObjectRef, FINGERPRINT_VERSION,
    KnowledgeSlot, KnowledgeState, PortableObject, PortablePayload, ProfileIdentity, Schema,
    SchemaBinding, SchemaFingerprint, SchemaTree, Table,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

include!("contracts_portable/runtime_harness.rs");
include!("contracts_portable/context_fixtures.rs");
include!("contracts_portable/export.rs");
include!("contracts_portable/live_import.rs");
include!("contracts_portable/profile_mapping.rs");
include!("contracts_portable/atomic_import.rs");
include!("contracts_portable/dbt_import.rs");
include!("contracts_portable/credential_skip.rs");
include!("contracts_portable/preview_and_parity.rs");
