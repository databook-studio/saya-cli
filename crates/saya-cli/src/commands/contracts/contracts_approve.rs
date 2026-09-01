//! the batch approve path: confirm every candidate in the profile's
//! bounded review queue, then report every item it refused.
//!
//! Split from `contracts_decide.rs` (the single-item path this parallels) to
//! keep that file under the size cap; the shared module `contracts.rs` was
//! already touched for dispatch. The batch adds no validation of its own: every
//! id goes through the same [`crate::contracts::confirm`] the decide path uses,
//! so a batch is expected to be a *mixture* — an item dismissed between the
//! queue read and the sweep, or one whose cached schema cannot vet it, is
//! refused. The failure mode this module exists to prevent is a summary that
//! hides those refusals, so outcomes are reported per item and the summary
//! names both counts.
//!
//! Consent: the queue is always printed first — the same lines `/queue`
//! prints — and without `--yes` nothing is approved (deny by default, the
//! precedent `--non-interactive` sets for approvals). With `--yes` the sweep
//! runs. The command never blocks on a prompt: the shared dispatcher also
//! serves the TUI session loop, where a blocking stdin read cannot work.

use super::{EXIT_CONTRACT_ERROR, cached_schema_availability, queue_item_view};
use crate::commands::output::{emit, failure_message, result};
use crate::contracts::{QUEUE_DEFAULT_LIMIT, approve_all, review_queue};
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::SqliteStateStore;
use saya_types::{ClaimId, ProfileIdentity};

/// An unreadable store is not an empty queue. Matches the wording the read
/// commands use so "nothing waiting" and "could not read" stay distinguishable.
const STORE_UNAVAILABLE_MSG: &str = "Local state store unavailable; contracts could not be read.";

/// Approves the profile's queued candidates. `profile_name` is the
/// human-facing name (renders); `identity` is what the store is read against —
/// the same pair `resolve_profile` produces, with the opaque identity kept out
/// of every message. Scope (Q1/Q3): the same bounded queue the user reads with
/// `contracts queue` — default limit [`QUEUE_DEFAULT_LIMIT`], `limit`
/// overridden and clamped inside [`review_queue`], active profile by default.
pub(super) async fn approve_queue(
    store: &SqliteStateStore,
    format: RenderFormat,
    profile_name: &str,
    identity: &ProfileIdentity,
    yes: bool,
    limit: Option<usize>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let limit = limit.unwrap_or(QUEUE_DEFAULT_LIMIT);
    let cached = cached_schema_availability(store, identity).await;
    let schema_pair = (identity.clone(), cached);
    let schemas = std::slice::from_ref(&schema_pair);
    let queued = match review_queue(store, std::slice::from_ref(identity), schemas, limit).await {
        Ok(queued) => queued,
        Err(_) => {
            return failure_message(EXIT_CONTRACT_ERROR, STORE_UNAVAILABLE_MSG.into(), format);
        }
    };
    // Invariant 4: the user sees the exact set before anything happens — the
    // same lines `/queue` shows, on every path, `--yes` or not.
    let items: Vec<_> = queued
        .iter()
        .map(|candidate| queue_item_view(candidate, profile_name))
        .collect();
    emit(
        TerminalEvent::ContractQueue {
            items: items.clone(),
        },
        format,
    );
    if queued.is_empty() {
        // A clean no-op, not an error (deliverable 6).
        return result("No candidates awaiting approval.".into(), format);
    }
    if !yes {
        // Deny by default, matching the approvals precedent: a script (or a
        // `--non-interactive` run) cannot bulk-confirm by accident. The
        // preview above is the "what would happen"; this line is the ask.
        let noun = if queued.len() == 1 {
            "candidate"
        } else {
            "candidates"
        };
        emit(
            TerminalEvent::Result {
                message: format!(
                    "Refusing to approve {n} {noun} without --yes. Rerun with --yes to approve them.",
                    n = queued.len()
                ),
            },
            format,
        );
        return Ok(EXIT_CONTRACT_ERROR);
    }
    let ids: Vec<ClaimId> = queued.iter().map(|c| c.claim.id.clone()).collect();
    let outcomes = approve_all(store, &ids).await;
    let mut approved = 0usize;
    let mut refused = 0usize;
    for (id, outcome) in &outcomes {
        match outcome {
            Ok(claim) => {
                approved += 1;
                // The same typed event the single-item confirm emits — the
                // batch reports per item in the vocabulary `contracts decide`
                // already uses, never one aggregated line.
                emit(
                    TerminalEvent::ContractChanged {
                        claim_id: claim.id.as_str().to_string(),
                        action: "confirmed".into(),
                        status: "confirmed".into(),
                    },
                    format,
                );
            }
            Err(error) => {
                refused += 1;
                let item = items.iter().find(|item| item.claim_id == id.as_str());
                let (kind, object) = item
                    .map(|item| (item.kind.as_str(), item.object.as_str()))
                    .unwrap_or(("", ""));
                // The refusal names the item and the reason `confirm` gave —
                // on stdout with the approvals so no adapter (the TUI captures
                // stdout or stderr, never both) can drop it.
                emit(
                    TerminalEvent::Result {
                        message: format!("refused {id}  {kind}  {object} — {error}"),
                    },
                    format,
                );
            }
        }
    }
    // The summary carries both counts; it is the footer to the per-item lines,
    // never a replacement for them.
    emit(
        TerminalEvent::Result {
            message: format!("approved {approved}, refused {refused} (profile: {profile_name})"),
        },
        format,
    );
    // A partial batch is a success (invariant 3). Only a sweep that approved
    // nothing is the failure a script must see in the exit status.
    if approved == 0 {
        Ok(EXIT_CONTRACT_ERROR)
    } else {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{capture_output_start, capture_output_take};
    use crate::render::RenderFormat;
    use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore};
    use saya_types::{
        ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
        KnowledgeState, SchemaBinding, SchemaTree,
    };

    fn identity() -> ProfileIdentity {
        ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
    }

    fn object_ref(profile: &ProfileIdentity, name: &str) -> DatabaseObjectRef {
        DatabaseObjectRef::new(
            profile.clone(),
            "catalog",
            "public",
            name,
            DatabaseObjectKind::Table,
        )
        .unwrap()
    }

    async fn store_at(root: &std::path::Path) -> SqliteStateStore {
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        // Touch the pool so migrations run; the identity is the validated shape.
        store
            .upsert_schema(identity().as_str(), &SchemaTree::default())
            .await
            .unwrap();
        store
    }

    async fn put_item(
        store: &SqliteStateStore,
        object: &DatabaseObjectRef,
        payload: ClaimPayload,
        state: KnowledgeState,
    ) -> ClaimId {
        let slot = KnowledgeSlot::TableAlias;
        let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
        let value = payload.clone();
        store
            .put_knowledge_item(KnowledgeItemRequest {
                object: object.clone(),
                slot,
                value: payload,
                source: if state == KnowledgeState::Active {
                    ClaimOrigin::UserExplicit
                } else {
                    ClaimOrigin::AssistantInferred
                },
                state,
                schema_binding_json: serde_json::to_string(&binding).unwrap(),
                fingerprint: crate::commands::unobserved_fingerprint(),
            })
            .await
            .unwrap();
        let id = store
            .knowledge_for_object(object)
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.value == value)
            .map(|item| item.id)
            .expect("item stored");
        ClaimId::parse(&id).unwrap()
    }

    async fn run(store: &SqliteStateStore, yes: bool) -> (i32, String, String) {
        capture_output_start();
        let code = approve_queue(store, RenderFormat::Text, "pagila", &identity(), yes, None)
            .await
            .unwrap();
        let (out, err) = capture_output_take();
        (code, out, err)
    }

    /// the batch-approve slice deliverable 5 at the command layer: a mixed batch with `--yes` —
    /// one candidate confirms (persisted), one is refused with its reason —
    /// reports the per-item outcomes and both counts, and exits 0 (a partial
    /// batch is a success, never a rollback). The refusal exercised here is the
    /// one reachable from a queue read: a candidate whose object the populated
    /// cached schema no longer names (`ObjectGone`). The `Dismissed` refusal
    /// (`Conflict`) is exercised at the operations layer
    /// (`contracts::tests::approve_all_reports_a_mixed_batch_and_persists_the_successes`)
    /// — a dismissed item never appears in a queue snapshot, so it can only
    /// reach the sweep through the stale-snapshot window `approve_all` models.
    #[tokio::test]
    async fn mixed_batch_reports_approvals_and_refusals_and_persists() {
        let root = std::env::temp_dir().join(format!(
            "saya-approve-mixed-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let profile = identity();
        // A populated cache naming only `orders` — so a candidate on `orders`
        // confirms and a candidate on `shipments` is refused as gone.
        let live = saya_types::SchemaTree {
            databases: vec![saya_types::Database {
                name: "catalog".into(),
                schemas: vec![saya_types::Schema {
                    name: "public".into(),
                    tables: vec![saya_types::Table {
                        name: "orders".into(),
                        columns: vec![saya_types::Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        }],
                    }],
                }],
            }],
        };
        store.upsert_schema(profile.as_str(), &live).await.unwrap();
        let obj = object_ref(&profile, "orders");
        let pending = put_item(
            &store,
            &obj,
            ClaimPayload::table_alias("orders").unwrap(),
            KnowledgeState::Pending,
        )
        .await;
        let gone_obj = object_ref(&profile, "shipments");
        let gone = put_item(
            &store,
            &gone_obj,
            ClaimPayload::table_alias("shipments").unwrap(),
            KnowledgeState::Pending,
        )
        .await;

        let (code, out, err) = run(&store, true).await;

        assert_eq!(code, 0, "a partial batch is a success; stderr: {err}");
        // The preview named the set first (invariant 4)...
        assert!(
            out.contains("catalog.public.orders"),
            "preview shown: {out}"
        );
        assert!(out.contains("shipments"), "preview shows both: {out}");
        //...the approval was reported per item...
        assert!(
            out.contains(&format!("confirmed {}", pending.as_str())),
            "approval line: {out}"
        );
        //...the refusal was reported per item, with the reason...
        assert!(
            out.contains(&format!("refused {}", gone.as_str())),
            "refusal line names the item: {out}"
        );
        assert!(
            out.contains("no longer in the schema"),
            "refusal line names why: {out}"
        );
        //...and the summary carries both counts.
        assert!(out.contains("approved 1, refused 1"), "summary: {out}");
        // The approval persisted despite the refusal (invariant 3).
        let state = store
            .get_knowledge_item(pending.as_str())
            .await
            .unwrap()
            .expect("item present")
            .state;
        assert_eq!(state, KnowledgeState::Active);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Without `--yes` the queue is printed and nothing is approved — the
    /// deny-by-default consent path, including under `--non-interactive`.
    #[tokio::test]
    async fn without_yes_the_queue_is_printed_and_nothing_is_approved() {
        let root = std::env::temp_dir().join(format!(
            "saya-approve-preview-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = store_at(&root).await;
        let profile = identity();
        let obj = object_ref(&profile, "orders");
        let pending = put_item(
            &store,
            &obj,
            ClaimPayload::table_alias("orders").unwrap(),
            KnowledgeState::Pending,
        )
        .await;

        let (code, out, _err) = run(&store, false).await;

        assert_eq!(code, EXIT_CONTRACT_ERROR, "refusal exits the contract code");
        assert!(
            out.contains("catalog.public.orders"),
            "preview shown: {out}"
        );
        assert!(
            out.contains("without --yes"),
            "the refusal says how to proceed: {out}"
        );
        assert!(!out.contains("confirmed "), "nothing was approved: {out}");
        let state = store
            .get_knowledge_item(pending.as_str())
            .await
            .unwrap()
            .expect("item present")
            .state;
        assert_eq!(state, KnowledgeState::Pending, "nothing was written");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// An empty queue is a clean no-op: exit 0, one line, nothing approved
    /// (deliverable 6 at the command layer).
    #[tokio::test]
    async fn empty_queue_is_a_clean_noop() {
        let root = std::env::temp_dir().join(format!(
            "saya-approve-empty-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = store_at(&root).await;

        let (code, out, _err) = run(&store, true).await;

        assert_eq!(code, 0);
        assert!(
            out.contains("No candidates awaiting approval"),
            "clean no-op message: {out}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
