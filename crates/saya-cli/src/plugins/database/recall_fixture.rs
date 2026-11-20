//! The harness the plugin's `context()` tests run over: a scratch store, an
//! identity-carrying registry, and the memory settings under test.
//!
//! The seeded claim comes from the store helpers the recall-context tests use,
//! included by path so the hook is exercised over exactly the store shape those
//! tests assert against rather than over a second, drifting fixture.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use saya_agent::{RunContext, RunSurface};
use saya_config::{MemoryMode, ResolvedMemory};
use saya_connectors::DatabaseConnector;
use saya_types::{
    ConnectionError, DatabaseProfile, ProfileIdentity, QueryRequest, QueryResult, SchemaTree,
    SqlDialect,
};

use super::DatabasePlugin;
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// The conversation byte budget these tests recall under — the same role the
/// loop's `context_byte_budget` plays in production.
pub(super) const BYTE_BUDGET: usize = 32 * 1024;

/// The question the seeded claim answers.
pub(super) const PROMPT: &str = "orders by month";

/// A chat run's context: the surface has a database, so the plugin activates.
pub(super) const CHAT: RunContext = RunContext {
    surface: RunSurface::Chat,
    database_profiles: 1,
};

#[allow(dead_code)] // the fixture carries helpers only some of its callers seed with
pub(super) mod seed {
    use saya_store::{SchemaStore, SqliteStateStore};
    use saya_types::{
        ClaimOrigin, ClaimPayload, Column, Database, DatabaseObjectKind, DatabaseObjectRef,
        KnowledgeSlot, KnowledgeState, ProfileIdentity, Schema, SchemaTree, Table,
    };

    include!("../../agent/recall_context_tests/fixtures/schema.rs");

    include!("../../agent/recall_context_tests/fixtures/claims.rs");

    /// Seeds the cached `orders` schema and one confirmed
    /// `default_time_column` claim under it, so a later recall reads the claim
    /// as current — the fixture's own acceptance scenario.
    pub(crate) async fn seed_confirmed_claim(store: &SqliteStateStore, identity: &ProfileIdentity) {
        let _ = seed_orders_with_created_at(store, identity).await;
    }
}

/// A connector that never touches a live database: recall reads the store, not
/// the connector, so the registry only needs an entry to carry an identity.
struct IdleConnector;

#[async_trait]
impl DatabaseConnector for IdleConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

/// A scratch directory under the workspace's own `target/` — never the system
/// temp directory.
pub(super) fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-plugins-database-recall-{name}"));
    std::fs::create_dir_all(&root).expect("scratch directory");
    root
}

pub(super) fn identity_for(name: &str) -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::Sqlite {
            path: "plugin-recall.sqlite".into(),
            read_only: true,
        },
        Path::new("/plugin-recall-test/connections.toml"),
    )
}

/// A registry whose single primary connection carries a real identity.
pub(super) fn registry_for(name: &str, identity: &ProfileIdentity) -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new(name);
    registry.insert(
        name,
        ConnectionEntry {
            connector: Box::new(IdleConnector),
            dialect: SqlDialect::Sqlite,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
}

/// The resolved `[memory]` settings: the mode under test, every bound at the
/// config defaults the recall path reads.
pub(super) fn memory(mode: MemoryMode) -> ResolvedMemory {
    ResolvedMemory {
        mode,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// The chat plugin the recall tests drive: the one `tests` builds, so both
/// files describe the same surface.
pub(super) fn chat_plugin() -> DatabasePlugin {
    super::tests::chat_plugin()
}

/// What a caller sees of a block: everything but the type's identity.
pub(super) fn blocks_view(blocks: &[saya_agent::ContextBlock]) -> Vec<(String, String, bool)> {
    blocks
        .iter()
        .map(|block| (block.label.clone(), block.body.clone(), block.truncated))
        .collect()
}
