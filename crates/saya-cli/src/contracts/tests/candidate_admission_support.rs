/// A recall request under `Confirmed` mode that admits one named candidate via
/// `use_candidate_once`. Everything else is the `recall_request` default: the
/// model-facing policy, fresh schemas, no explicit refs.
fn recall_admitting_one<'a>(
    profiles: &'a [ProfileIdentity],
    schemas: &'a [(ProfileIdentity, SchemaAvailability)],
    terms: &'a [String],
    admit: Option<ClaimId>,
) -> RecallRequest<'a> {
    RecallRequest {
        profiles,
        explicit_refs: &[],
        terms,
        allow_database_context: true,
        schemas,
        now_unix_ms: FRESH_NOW,
        bounds: RecallBounds::defaults(),
        recall_mode: RecallMode::Confirmed,
        policy: RetrievalPolicy::ForModel,
        admit_candidate: admit,
    }
}

/// Seeds a `Pending` knowledge item (a candidate) the recall path reads, and
/// returns its store-assigned `ki-` id. Used by the recall-admission tests
/// (and by `use_candidate_once`, which now reads the same `knowledge_items`
/// table). The schema names the table and `amount` so the item's binding
/// (`Column { amount, Exists }`) classifies `current`.
/// Proposes a candidate claim with `AssistantInferred` origin and no evidence,
/// returning its id. Mirrors how `contract_propose` stores a candidate: the
/// model inferred it, no human confirmed it, no observation supports it yet.
async fn seed_one_pending_item(
    store: &SqliteStateStore,
) -> (ProfileIdentity, DatabaseObjectRef, ClaimId) {
    use saya_store::KnowledgeItemStore;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let tree = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    store.upsert_schema(p.as_str(), &tree).await.unwrap();
    put_item(
        store,
        &obj,
        ClaimPayload::column_description("amount", "how much").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = ClaimId::parse(
        &store
            .knowledge_for_object(&obj)
            .await
            .expect("knowledge items listed")
            .into_iter()
            .find(|i| {
                i.slot
                    == KnowledgeSlot::ColumnDescription {
                        column: "amount".into(),
                    }
            })
            .expect("pending item stored")
            .id,
    )
    .expect("ki id");
    (p, obj, id)
}
