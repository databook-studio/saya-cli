//! Tests for the preference projection: a worktree's explicit preferences
//! reach the request as one bounded, untrusted, labelled context block on the
//! user turn — after the instruction blocks, never in the system prompt,
//! never from another worktree, independent of the database privacy gate.

use super::{
    MAX_PREFERENCE_REQUEST_BYTES, project_preferences, remaining_request_bytes, render_preferences,
};
use crate::agent::context::sources::UNTRUSTED_PREFIX;
use crate::agent::runtime::run_prompt_with_inputs;
use crate::agent::turn_inputs::TurnInputs;
use crate::commands::preferences::worktree_id_for;
use crate::config::runtime::RuntimeConfig;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use crate::interactive::session::universe::SessionUniverse;
use async_trait::async_trait;
use saya_agent::{
    AgentMode, AgentOutput, ApprovalPolicy, CancellationToken, ChatMessage, ChatProvider,
    ChatRequest, ChatResponse, ContextBlock, NoopEventSink, render_untrusted_block, turn_bytes,
};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_connectors::DatabaseConnector;
use saya_store::{ProjectPreferenceStore, SqliteStateStore};
use saya_types::{
    ConnectionError, Database, DatabaseProfile, PROJECT_PREFERENCE_VERSION, PreferenceId,
    PreferenceSource, ProfileIdentity, ProjectPreference, QueryRequest, QueryResult, Schema,
    SchemaTree, SqlDialect, Table, WorktreeId,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const CLOSE: &str = "<<<CONTEXT_BLOCK_END>>>";
const OPEN: &str = "<<<CONTEXT_BLOCK_BEGIN>>>";
const ESCAPED_CLOSE: &str = "<<<\\CONTEXT_BLOCK_END>>>";
const HEADER: &str =
    "Explicit project preferences confirmed by the user for this worktree (oldest first):";

fn stamp(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "saya-pref-ctx-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// The exact post-escape, post-wrapper request bytes one block adds.
fn cost_of(block: &ContextBlock) -> usize {
    turn_bytes(None, std::slice::from_ref(block), "").saturating_sub(turn_bytes(None, &[], ""))
}

/// A hand-built preference, for the pure renderer: the store refuses
/// secret-shaped text at add time, so the redaction pass is exercised here.
fn item(text: &str, created: i64, tag: usize) -> ProjectPreference {
    ProjectPreference {
        version: PROJECT_PREFERENCE_VERSION,
        id: PreferenceId::parse(&format!("pp-{tag:032x}")).expect("the id matches the pp- shape"),
        worktree: WorktreeId::from_canonical_path("/saya-prefs-test").expect("the worktree shape"),
        text: text.to_owned(),
        source: PreferenceSource::UserExplicit,
        created_unix_ms: created,
        updated_unix_ms: created,
    }
}

async fn seed(store: &SqliteStateStore, worktree: &WorktreeId, text: &str, now: i64) {
    store
        .add_project_preference(worktree, text, now)
        .await
        .expect("the store accepts the seeded text");
}

/// A unit-test fixture: the stamp root, created, plus its canonical path —
/// the exact string the worktree identity hashes.
fn canonical_fixture(tag: &str) -> (PathBuf, PathBuf) {
    let root = stamp(tag);
    std::fs::create_dir_all(&root).expect("create the fixture root");
    let canonical = std::fs::canonicalize(&root).expect("the fixture dir canonicalises");
    (root, canonical)
}

include!("preferences_tests/absence_and_format.rs");

include!("preferences_tests/budget.rs");

// ===========================================================================
// The runtime wiring: the real turn path splices the preferences block into
// the request's user turn, after the instruction blocks.
// ===========================================================================

include!("preferences_tests/runtime_config.rs");
include!("preferences_tests/runtime_provider.rs");
include!("preferences_tests/runtime_turn.rs");
