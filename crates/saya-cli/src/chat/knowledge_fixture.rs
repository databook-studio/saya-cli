//! The live chat turn the knowledge-event twins run over.
//!
//! A composed session with one SQLite database, its memory mode stated, an
//! `orders` table in the database the turn's statements read, and — when the
//! test asks for one — a `default_time_column` claim seeded into the store the
//! plugin recalls from, filed under the identity the session's connection
//! carries. The store's cached schema names the claimed column, so the claim
//! reaches the model as `current`.
//!
//! [`Recording`] is the front's own sink: every event the turn emitted, in
//! order, plus the provider's arrival count at the instant the first
//! `KnowledgeSupplied` was emitted — how the ordering twin proves the emit
//! preceded the request.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::{AgentOutput, ApprovalPolicy, ChatProvider};
use saya_config::MemoryMode;
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, Database, DatabaseObjectKind,
    DatabaseObjectRef, KnowledgeSlot, KnowledgeState, ProfileIdentity, Schema, SchemaBinding,
    SchemaTree, Table,
};

use super::compose::RunComposition;
use super::compose_fixture::{self, Answering, Composed, RootGuard};
use super::compose_recall_fixture::identity_of;
use super::knowledge_sinks::{Extractor, Recording};
use crate::agent::tools::consent::Answer;
use crate::commands::coding::CodingError;
use crate::connection::ConnectionRegistry;

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The user turn whose terms select the seeded claim: it names the table the
/// claim is filed under, at the name tier recall ranks highest. It states the
/// word `table` too, so the turn shows database work and the plugin's family —
/// and with it the knowledge line — is advertised from this turn
/// (`plugins::database::family`).
pub(super) const QUESTION: &str = "the orders table by month";

/// The claimed time column, and the different time-named column the
/// contradicting statement references instead.
pub(super) const CLAIMED_COLUMN: &str = "return_date";
pub(super) const OTHER_COLUMN: &str = "rental_date";

/// The statement that contradicts the claim: another time-named column on the
/// claimed object, the claimed column absent.
pub(super) const OVERRIDE_SQL: &str =
    "SELECT rental_date FROM orders WHERE rental_date > '2024-01-01'";

/// The statement that honours it: the claimed column is the one referenced.
pub(super) const HONOURING_SQL: &str =
    "SELECT return_date FROM orders WHERE return_date > '2024-01-01'";

/// Unparseable to the detector, which then fails closed: no finding, no guess.
pub(super) const UNPARSEABLE_SQL: &str = "SELECT FROM WHERE";

/// An extractor reply proposing nothing: the turn learns nothing new.
pub(super) const NO_PROPOSALS: &str = r#"{"proposals": []}"#;

/// What a test states about the session: its memory mode, whether the privacy
/// gate is open, the claim the store holds, and the extractor's reply.
pub(super) struct Wanted {
    pub(super) memory: MemoryMode,
    pub(super) claim: Option<ClaimStatus>,
    pub(super) allow_query_data: bool,
    /// The post-turn extractor's reply. `None` attaches no learning source at
    /// all, so the turn learns nothing and announces no override finding.
    pub(super) extractor: Option<String>,
}

impl Default for Wanted {
    fn default() -> Self {
        Self {
            memory: MemoryMode::Assisted,
            claim: Some(ClaimStatus::Confirmed),
            allow_query_data: true,
            extractor: Some(NO_PROPOSALS.to_owned()),
        }
    }
}

/// One composed session's run, with the folder that must outlive it.
pub(super) struct Case {
    pub(super) composition: RunComposition,
    claim: Option<ClaimId>,
    /// The opaque identity the session's connection carries: a hash over
    /// connection material that must never reach an event.
    identity: ProfileIdentity,
    /// The qualified name of the object the claim is filed under, as the live
    /// connection names it.
    object: String,
    pub(super) extractor: Arc<Extractor>,
    root: RootGuard,
}

impl Case {
    /// The seeded claim's store id, so an event assertion names exactly what
    /// recall supplied.
    pub(super) fn claim_id(&self) -> ClaimId {
        self.claim.clone().expect("the case seeded a claim")
    }

    /// The opaque profile identity the session's connection carries.
    pub(super) fn identity(&self) -> ProfileIdentity {
        self.identity.clone()
    }

    /// The qualified name of the object the seeded claim is filed under.
    pub(super) fn object(&self) -> &str {
        &self.object
    }

    /// The run's next turn, asked with `prompt`, its events landing in `sink`:
    /// the turn's own outcome, waited for under the fixture's load ceiling.
    pub(super) async fn turn(
        &self,
        sink: &Recording,
        prompt: &str,
    ) -> Result<AgentOutput, CodingError> {
        let sink = sink.sink();
        tokio::time::timeout(
            RUN,
            self.composition.job().run_next_turn(sink.as_ref(), prompt),
        )
        .await
        .expect("the turn settles")
    }

    /// Makes the store unavailable at the one read recall makes: the knowledge
    /// table is dropped, so recall's read errors while every table the run
    /// itself writes stays intact.
    pub(super) async fn make_store_unavailable(&self) {
        let path = self.root.state_store_path();
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display()))
            .await
            .expect("the fixture store opens a second connection");
        sqlx::query("DROP TABLE IF EXISTS knowledge_items")
            .execute(&pool)
            .await
            .expect("the fixture drops the knowledge table");
        pool.close().await;
    }
}

/// Composes the session's run over a scratch folder and opens it — asking
/// nobody: the session's mode is the consent to open.
pub(super) async fn build(name: &str, endpoint: &str, wanted: Wanted) -> Case {
    let Wanted {
        memory,
        claim,
        allow_query_data,
        extractor,
    } = wanted;
    let fixture = compose_fixture::in_place_with_memory(name, endpoint, memory).await;
    // A session answer to each call: the turn's SQL is approved as the
    // session's own, so a query the model asks for actually runs.
    let front = Answering::new(Answer::Session, Answer::Session);
    let Composed {
        application,
        mut run,
        root,
    } = fixture.into_parts(front.clone(), name, ApprovalPolicy::Ask);
    run.memory = memory;
    run.memory_reachable = memory != MemoryMode::Off;
    run.allow_query_data = allow_query_data;
    let extractor = Arc::new(Extractor::new(extractor));
    if memory != MemoryMode::Off && extractor.attached() {
        run.learning = Some(Arc::clone(&extractor) as Arc<dyn ChatProvider>);
    }
    let identity = identity_of(&run.connections);
    seed_orders_table(&root.database_path()).await;
    // The claim is filed under the names the live connection reports, so the
    // extraction's own resolution of the object it observed agrees with what
    // recall supplied — the anti-self-reinforcement match is by qualified name.
    let names = live_names(&run.connections).await;
    let object = format!("{}.{}.orders", names.0, names.1);
    let claim = match claim {
        Some(status) => Some(seed_claim(application.store(), &identity, status, &names).await),
        None => None,
    };
    let composition = RunComposition::compose(application, run)
        .await
        .expect("the session's run composes");
    composition.open().await.expect("the run opens");
    Case {
        composition,
        claim,
        identity,
        object,
        extractor,
        root,
    }
}

/// The database and schema names the session's live connection reports: the
/// names a statement's object resolves to.
async fn live_names(connections: &ConnectionRegistry) -> (String, String) {
    let (_, entry) = connections
        .entries()
        .into_iter()
        .next()
        .expect("the fixture connected one database");
    let tree = entry
        .connector
        .schema()
        .await
        .expect("the fixture database reports its schema");
    let database = tree
        .databases
        .first()
        .map(|database| database.name.clone())
        .unwrap_or_else(|| "main".to_owned());
    let schema = tree
        .databases
        .first()
        .and_then(|database| database.schemas.first())
        .map(|schema| schema.name.clone())
        .unwrap_or_else(|| "main".to_owned());
    (database, schema)
}

/// Seeds the cached `orders` schema carrying the claimed column, and the
/// `default_time_column` claim under it — the shape recall reads as `current`.
async fn seed_claim(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
    status: ClaimStatus,
    names: &(String, String),
) -> ClaimId {
    let (database_name, schema_name) = names;
    let object = DatabaseObjectRef::new(
        identity.clone(),
        database_name,
        schema_name,
        "orders",
        DatabaseObjectKind::Table,
    )
    .expect("the fixture's object reference is well formed");
    let tree = SchemaTree {
        databases: vec![Database {
            name: database_name.clone(),
            schemas: vec![Schema {
                name: schema_name.clone(),
                tables: vec![Table {
                    name: "orders".into(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: vec![
                        Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        },
                        Column {
                            name: CLAIMED_COLUMN.into(),
                            data_type: "timestamp".into(),
                            nullable: false,
                        },
                    ],
                }],
            }],
        }],
    };
    store
        .upsert_schema(identity.as_str(), &tree)
        .await
        .expect("the fixture caches the orders schema");
    let payload = ClaimPayload::default_time_column(CLAIMED_COLUMN, None)
        .expect("the fixture's claim payload is well formed");
    let slot = KnowledgeSlot::TableDefaultTime;
    let binding = SchemaBinding::derive(&slot, &payload).expect("the slot and payload agree");
    let request = KnowledgeItemRequest {
        object: object.clone(),
        slot,
        value: payload,
        source: match status {
            ClaimStatus::Candidate => ClaimOrigin::AssistantInferred,
            _ => ClaimOrigin::UserExplicit,
        },
        state: match status {
            ClaimStatus::Confirmed => KnowledgeState::Active,
            ClaimStatus::Candidate => KnowledgeState::Pending,
            _ => KnowledgeState::Dismissed,
        },
        schema_binding_json: serde_json::to_string(&binding).expect("the binding serialises"),
        fingerprint: crate::commands::unobserved_fingerprint(),
    };
    store
        .put_knowledge_item(request)
        .await
        .expect("the fixture stores the claim");
    // The store assigns the id; read it back so an assertion compares against
    // exactly what recall supplied.
    ClaimId::parse(
        &store
            .knowledge_for_object(&object)
            .await
            .expect("the fixture lists the object's knowledge")
            .into_iter()
            .find(|item| item.slot == KnowledgeSlot::TableDefaultTime)
            .expect("the default_time item is stored")
            .id,
    )
    .expect("the store's id parses")
}

/// Adds the `orders` table the turn's statements read: the claimed time column
/// and the different time-named one the contradicting statement reaches for.
async fn seed_orders_table(database: &std::path::Path) {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", database.display()))
        .await
        .expect("the fixture's database opens a second connection");
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS orders (id INTEGER, return_date TEXT, rental_date TEXT)",
    )
    .execute(&pool)
    .await
    .expect("the fixture creates the orders table");
    sqlx::query("INSERT INTO orders VALUES (1, '2024-02-01', '2024-01-15')")
        .execute(&pool)
        .await
        .expect("the fixture seeds one order");
    pool.close().await;
}
