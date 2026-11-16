use super::{provider::Provider, sqlite_fixture::SelectedSqlite, support::Fixture};
use saya_agent::CancellationToken;
use saya_cli::{CodingAuthorization, CodingError};
use saya_store::{CodingBudgetReservationRecord, ManagedCheckoutRecord};
use saya_types::JobId;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) struct History {
    pub(super) results: BTreeMap<String, Value>,
    pub(super) calls: BTreeMap<String, (String, Value)>,
}

pub(super) fn parse_history(bodies: &[String]) -> History {
    assert!(bodies.iter().any(|body| {
        [
            "stage2-read-a",
            "stage2-read-b",
            "stage2-read-c",
            "stage2-read-d",
        ]
        .iter()
        .all(|id| body.contains(id))
    }));
    for body in bodies {
        let body: Value = serde_json::from_str(body).unwrap();
        assert!(
            body["tools"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|tool| tool["function"]["name"] != "coding_apply"),
            "apply must not be advertised in a native Stage 2 request"
        );
    }
    let results: BTreeMap<String, Value> = bodies
        .iter()
        .flat_map(|body| {
            let body: Value = serde_json::from_str(body).unwrap();
            body["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|message| {
                    let id = message["tool_call_id"].as_str()?;
                    serde_json::from_str::<Value>(message["content"].as_str()?)
                        .ok()
                        .map(|value| (id.to_owned(), value))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut calls = BTreeMap::new();
    for body in bodies {
        let body: Value = serde_json::from_str(body).unwrap();
        for message in body["messages"].as_array().into_iter().flatten() {
            for call in message["tool_calls"].as_array().into_iter().flatten() {
                let id = call["id"].as_str().unwrap().to_owned();
                let name = call["function"]["name"].as_str().unwrap().to_owned();
                let arguments =
                    serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap())
                        .unwrap();
                if let Some(previous) = calls.insert(id.clone(), (name.clone(), arguments.clone()))
                {
                    assert_eq!(
                        previous,
                        (name, arguments),
                        "tool id {id} changed in history"
                    );
                }
            }
        }
    }
    for (id, name) in [
        ("stage2-read-a", "read-a.txt"),
        ("stage2-read-b", "read-b.txt"),
        ("stage2-read-c", "read-c.txt"),
        ("stage2-read-d", "read-d.txt"),
    ] {
        let result = &results[id];
        assert_eq!(result["path"], name);
        assert_eq!(result["content"], format!("     1→{name}\n"));
        // A false result flag is left out of the shape, so an absent
        // `truncated` reads as that default: the whole file came back.
        assert!(
            !result["truncated"].as_bool().unwrap_or(false),
            "a whole read is not truncated: {result}"
        );
    }
    assert!(
        calls.values().all(|(name, _)| name != "coding_apply"),
        "apply must not be invoked"
    );
    History { results, calls }
}

/// Helpers share the root's working checkout W, so the job holds exactly
/// two checkouts: W, and the prepared candidate the owner assembled from it.
/// Returns the working checkout and the prepared candidate.
pub(super) fn direct_checkouts(
    checkouts: &[ManagedCheckoutRecord],
) -> (&ManagedCheckoutRecord, &ManagedCheckoutRecord) {
    assert_eq!(
        checkouts.len(),
        2,
        "root working checkout plus prepared candidate"
    );
    let working = checkouts
        .iter()
        .find(|item| {
            item.ownership.agent_id().is_none()
                && item
                    .ownership
                    .managed_checkout_id()
                    .as_str()
                    .starts_with("work-")
        })
        .expect("the root working checkout W");
    let candidate = checkouts
        .iter()
        .find(|item| {
            item.ownership.agent_id().is_none()
                && item
                    .ownership
                    .managed_checkout_id()
                    .as_str()
                    .starts_with("candidate-")
        })
        .expect("the prepared candidate checkout");
    (working, candidate)
}

/// The direct root has no `coding_review` root tool: it prepares the candidate
/// from W and the owner reviews the prepared candidate. The review claims are
/// asserted through the operator path the `coding review` command drives.
pub(super) async fn assert_reviewed_prepared_candidate(
    resumed: &saya_cli::NativeCodingJob,
    candidate: &saya_types::ManagedCheckoutId,
) {
    let review = resumed
        .review_existing(candidate)
        .await
        .unwrap()
        .preview()
        .clone();
    assert_eq!(
        review["changes"],
        serde_json::json!([{"path":"tracked.txt","kind":"Modified","bytes":23}])
    );
    assert_eq!(review["apply_requires_fresh_approval"], true);
    assert!(
        review["source_sha256"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64)
    );
}

pub(super) async fn assert_database(result: &Value, selected: &SelectedSqlite) {
    assert_eq!(result["rows"].as_array().unwrap().len(), 1);
    assert_eq!(result["truncated"], true);
    assert!(result.to_string().contains("approved-row"));
    assert!(!result.to_string().contains("second-row"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM saya_sentinel")
            .fetch_one(&selected.pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM saya_sentinel ORDER BY rowid")
            .fetch_all(&selected.pool)
            .await
            .unwrap(),
        vec!["approved-row", "second-row"]
    );
    assert!(!selected.elsewhere.exists());
}

pub(super) async fn assert_resume_boundary(
    fixture: &Fixture,
    job: &JobId,
    start: &CodingAuthorization,
    checkouts: &[ManagedCheckoutRecord],
    reservations: &[CodingBudgetReservationRecord],
    provider: &Provider,
    body_count: usize,
) -> std::sync::Arc<saya_cli::NativeCodingJob> {
    let checkout_ids = checkouts
        .iter()
        .map(|item| item.ownership.managed_checkout_id().as_str().to_owned())
        .collect::<Vec<_>>();
    let reservation_sequences = reservations
        .iter()
        .map(|record| record.reservation.reservation_sequence().get())
        .collect::<Vec<_>>();
    assert!(matches!(
        fixture
            .application
            .resume(job, start, CancellationToken::new())
            .await,
        Err(CodingError::ApprovalRequired)
    ));
    assert_durable_identity(fixture, job, &checkout_ids, &reservation_sequences).await;
    assert_eq!(provider.bodies.lock().unwrap().len(), body_count);

    let proposal = fixture.application.propose_resume(job).await.unwrap();
    let fresh = CodingAuthorization::scoped(&proposal).unwrap();
    let resumed = fixture
        .application
        .resume(job, &fresh, CancellationToken::new())
        .await
        .expect("fresh resume approval reattaches the completed job without a provider run");
    assert_durable_identity(fixture, job, &checkout_ids, &reservation_sequences).await;
    assert_eq!(provider.bodies.lock().unwrap().len(), body_count);
    resumed
}

async fn assert_durable_identity(
    fixture: &Fixture,
    job: &JobId,
    checkout_ids: &[String],
    reservation_sequences: &[u64],
) {
    assert_eq!(
        fixture
            .store
            .list_managed_checkouts(job, None, 8)
            .await
            .unwrap()
            .items
            .iter()
            .map(|item| item.ownership.managed_checkout_id().as_str().to_owned())
            .collect::<Vec<_>>(),
        checkout_ids
    );
    assert_eq!(
        fixture
            .store
            .list_coding_budget_reservations(job, None, 100)
            .await
            .unwrap()
            .items
            .iter()
            .map(|record| record.reservation.reservation_sequence().get())
            .collect::<Vec<_>>(),
        reservation_sequences
    );
}
