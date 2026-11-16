use super::{fixture::Fixture, provider::is_child};
use saya_cli::NativeCodingJob;
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue, JobState};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) fn child_requests(bodies: &[Value]) -> Vec<Value> {
    bodies
        .iter()
        .filter(|body| is_child(body))
        .cloned()
        .collect()
}
pub(super) fn results(bodies: &[Value]) -> BTreeMap<String, String> {
    bodies
        .iter()
        .flat_map(|body| {
            body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|message| {
                    Some((
                        message["tool_call_id"].as_str()?.to_owned(),
                        message["content"].as_str()?.to_owned(),
                    ))
                })
        })
        .collect()
}
/// The file's own bytes behind the numbered rendering: drop the six-column
/// number and `→` that begins each delivered line.
pub(super) fn unnumbered(content: &str) -> String {
    content
        .split_inclusive('\n')
        .map(|line| line.split_once('→').map_or(line, |(_, text)| text))
        .collect()
}

pub(super) fn assert_valid_read_value(value: &Value, fixture: &Fixture, index: usize, id: &str) {
    let path = format!("padding-{index}.txt");
    let body = fixture.file_body(index);
    let expected_digest = format!("{:x}", Sha256::digest(body.as_bytes()));
    assert!(value.get("error").is_none(), "read failed: {id}: {value}");
    assert_eq!(value["path"], path, "{id}");
    assert_eq!(value["size"], body.len(), "{id}");
    assert_eq!(value["digest"], expected_digest, "{id}");
    let content = unnumbered(value["content"].as_str().expect("read content is text"));
    assert!(
        body.starts_with(&content),
        "read is an exact source prefix: {id}"
    );
    // A false result flag is left out of the shape, so an absent `truncated`
    // reads as that default: the read was not clipped.
    let truncated = value["truncated"].as_bool().unwrap_or(false);
    if truncated {
        assert!(
            content.len() < body.len(),
            "truncated read is a strict prefix: {id}"
        );
    } else {
        assert_eq!(
            content, body,
            "complete read returns all source bytes: {id}"
        );
    }
}
pub(super) fn content_bytes(body: &Value) -> usize {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["content"].as_str().map_or(0, str::len))
        .sum()
}
pub(super) async fn healthy(fixture: &Fixture, job: &NativeCodingJob, tools: usize) {
    let children = fixture
        .store
        .list_coding_children(job.job_id(), None, 10)
        .await
        .unwrap();
    assert_eq!(children.items.len(), 1);
    assert_eq!(children.items[0].state, JobState::Completed);
    let root = fixture
        .store
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(root.state, JobState::Completed);
    let mut cursor = None;
    let mut charged = 0;
    loop {
        let page = fixture
            .store
            .list_coding_budget_reservations(job.job_id(), cursor, 100)
            .await
            .unwrap();
        for record in page.items {
            let settlement = record.settlement.as_ref().unwrap();
            assert_eq!(settlement.effect_state(), BudgetEffectState::Settled);
            assert!(
                settlement
                    .observations()
                    .iter()
                    .all(|observation| observation.value() != BudgetObservationValue::Unknown),
                "no settled unknown effects"
            );
            if record.reservation.agent_id().is_some() {
                charged += record
                    .reservation
                    .charges()
                    .iter()
                    .filter(|charge| charge.dimension() == BudgetDimension::ToolCalls)
                    .map(|charge| charge.reserved().get())
                    .sum::<u64>();
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(charged, tools as u64);
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0
    );
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
    assert!(!fixture.repository.join("new.txt").exists());
    for index in 0..16 {
        assert_eq!(
            std::fs::read_to_string(fixture.repository.join(format!("padding-{index}.txt")))
                .unwrap(),
            fixture.file_body(index)
        );
    }
}
/// The direct root has no `coding_review` and no prepare tool: the owner
/// prepares the candidate from the root's own working checkout W at the end of
/// the run. The claim is unchanged in substance: a real source digest binds a
/// candidate that actually carries the child's `new.txt` change, and the
/// receipt is durable for that generation.
pub(super) async fn reviewed(fixture: &Fixture, job: &NativeCodingJob) {
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 8)
        .await
        .unwrap();
    let candidate = checkouts
        .items
        .iter()
        .find(|record| {
            record
                .ownership
                .managed_checkout_id()
                .as_str()
                .starts_with("candidate-")
        })
        .unwrap_or_else(|| panic!("the owner prepared a candidate at the end of the run"));
    let id = candidate.ownership.managed_checkout_id().as_str();
    let digest = id.trim_start_matches("candidate-");
    assert_eq!(
        digest.len(),
        64,
        "the candidate carries a source digest: {id}"
    );
    assert!(
        fixture
            .root
            .join("coding")
            .join(job.job_id().as_str())
            .join(format!("prepared-candidate-{digest}.json"))
            .exists(),
        "the receipt is durable for this generation"
    );
    assert_eq!(
        std::fs::read_to_string(
            std::path::Path::new(candidate.ownership.managed_path().as_str()).join("new.txt")
        )
        .unwrap(),
        "owned context change\n",
        "the prepared candidate carries the child change"
    );
}
