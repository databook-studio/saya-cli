//! The plan gate as the start path builds it. Plan mode is opt-in: a job that
//! never stated `--plan` composes no gate and writes the same request bytes it
//! always did. Where `--plan` is stated it is refused unless someone can answer
//! the submission and there is a write surface to open — a headless run cannot
//! answer, and a job with no `--write` path has nothing to open — so plan mode
//! is never silently dropped and never auto-approved.

use std::sync::Arc;

use async_trait::async_trait;
use saya_types::JobId;

use super::request_fixture::fixture_with;
use super::{CodingError, CodingJobRequest, create::plan_gate};
use crate::agent::tools::consent::Operator;

/// The bytes the fixture job's `request.json` held before plan mode existed —
/// every empty field skipped, and no `plan` key among them. A job that never
/// stated `--plan` must still write exactly these.
const LEGACY_REQUEST_BYTES: &[u8] = br#"{"job_id":"request-mode-legacy-resume","task":"resume a request written by an older binary","write_paths":["tracked.txt"],"database_profiles":[],"database_identities":{}}"#;

/// An operator that is never asked: every refusal below happens before the job
/// exists, so nothing can reach one.
struct Unasked;

#[async_trait]
impl Operator for Unasked {
    async fn ask(&self, _subject: &str, _arguments: &serde_json::Value) -> bool {
        unreachable!("the plan gate is built, or refused, before any prompt")
    }
}

fn request(plan: bool, write_paths: &[&str]) -> CodingJobRequest {
    CodingJobRequest::new(
        JobId::parse("request-mode-legacy-resume").unwrap(),
        "resume a request written by an older binary".into(),
        write_paths.iter().map(|path| (*path).to_owned()).collect(),
    )
    .unwrap()
    .with_plan(plan)
    .unwrap()
}

/// `--plan` on a run with no interactive surface is refused at start: the
/// operator is the only thing that can open the gate, so a job that could never
/// be answered is never created.
#[test]
fn a_headless_start_refuses_plan_mode_with_no_operator() {
    assert!(matches!(
        plan_gate(&request(true, &["tracked.txt"]), None),
        Err(CodingError::PlanNeedsOperator)
    ));
}

/// `--plan` with no `--write` path is refused at start: the gate opens a write
/// surface, and this job states none, so there would be nothing to open.
#[test]
fn plan_mode_with_no_write_path_is_refused() {
    let operator: Arc<dyn Operator> = Arc::new(Unasked);
    assert!(matches!(
        plan_gate(&request(true, &[]), Some(&operator)),
        Err(CodingError::PlanNeedsWritePath)
    ));
}

/// `--plan` with a `[tools] allow` list that leaves `submit_plan` out is
/// refused at start: the posture would have no way to end, so the job would
/// simply write and plan mode would have gated nothing.
#[test]
fn plan_mode_with_a_tool_list_that_hides_the_gate_is_refused() {
    let operator: Arc<dyn Operator> = Arc::new(Unasked);
    let gated = request(true, &["tracked.txt"])
        .with_tools_allow(Some(vec!["read".into(), "write".into()]))
        .unwrap();
    assert!(matches!(
        plan_gate(&gated, Some(&operator)),
        Err(CodingError::PlanNeedsGateTool)
    ));
    // The same list without `--plan` is a job like any other.
    let ungated = request(false, &["tracked.txt"])
        .with_tools_allow(Some(vec!["read".into(), "write".into()]))
        .unwrap();
    assert!(matches!(plan_gate(&ungated, None), Ok(None)));
}

/// A job that never stated `--plan` needs neither an operator nor a write path:
/// it composes no gate at all.
#[test]
fn a_job_without_plan_mode_composes_no_gate() {
    assert!(matches!(plan_gate(&request(false, &[]), None), Ok(None)));
}

/// `--plan` with both an operator and a write path composes the gate, closed:
/// the first request's advertisement is the read surface.
#[test]
fn plan_mode_with_an_operator_and_a_write_path_composes_a_closed_gate() {
    let operator: Arc<dyn Operator> = Arc::new(Unasked);
    let gate = plan_gate(&request(true, &["tracked.txt"]), Some(&operator))
        .unwrap()
        .expect("plan mode composes a gate");
    assert!(!gate.is_open(), "a fresh gate is closed");
}

/// The request bytes a job without plan mode writes are the bytes it always
/// wrote: the new field is skipped when it is false.
#[cfg(unix)]
#[tokio::test]
async fn a_request_without_plan_mode_writes_the_same_bytes() {
    let fixture = fixture_with("plan-absent-bytes", |request| request).await;
    let written = std::fs::read(fixture.job.records.join("request.json")).unwrap();
    assert_eq!(
        written, LEGACY_REQUEST_BYTES,
        "a job without plan mode writes the same request.json"
    );
}

/// A plan-first request round-trips: the field is written when it is set and
/// read back by resume.
#[test]
fn a_plan_request_round_trips_through_request_json() {
    let planned = serde_json::to_value(request(true, &["tracked.txt"])).unwrap();
    assert_eq!(planned.get("plan"), Some(&serde_json::Value::Bool(true)));
    let parsed: CodingJobRequest = serde_json::from_value(planned).unwrap();
    assert!(parsed.plan, "resume reads the posture back");

    let plain = serde_json::to_value(request(false, &["tracked.txt"])).unwrap();
    assert!(
        plain.get("plan").is_none(),
        "a job without plan mode carries no plan key: {plain}"
    );
}
