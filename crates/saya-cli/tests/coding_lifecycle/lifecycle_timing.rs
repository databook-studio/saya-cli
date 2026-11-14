//! The `saya start` lifecycle: bounded timing spans land in the job's private
//! records, and a headless start defers the candidate preparation nobody is
//! waiting for while W stays applyable on demand.

use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest, NativeCodingJob};
use saya_types::{JobId, ManagedCheckoutId};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc, time::Duration};

const FILES: usize = 9;

fn records(fixture: &Fixture, job: &NativeCodingJob) -> PathBuf {
    fixture.root.join("coding").join(job.job_id().as_str())
}

fn spans(fixture: &Fixture, job: &NativeCodingJob) -> Vec<Value> {
    let bytes = std::fs::read(records(fixture, job).join("lifecycle.jsonl"))
        .expect("the run records its lifecycle spans");
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).expect("each span is JSON"))
        .collect()
}

fn phase<'a>(spans: &'a [Value], name: &str) -> &'a Value {
    spans
        .iter()
        .find(|span| span["phase"] == name)
        .unwrap_or_else(|| panic!("the run records the {name} phase: {spans:?}"))
}

/// Runs the `start` entry over a small repository: the fixture's tracked file
/// plus `FILES` more, so the baseline inventory has a realistic shape.
async fn run_start(
    label: &str,
) -> (
    Fixture,
    Provider,
    Arc<NativeCodingJob>,
    saya_agent::AgentOutput,
) {
    let provider = Provider::script(false, 113);
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    for index in 0..FILES {
        std::fs::write(
            fixture.repository.join(format!("file-{index:02}.txt")),
            format!("file {index}\n"),
        )
        .unwrap();
    }
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Write the two assigned files".into(),
        vec!["turn-one.txt".into(), "turn-two.txt".into()],
    )
    .unwrap();
    let (proposed, plan) = fixture
        .application
        .propose_with_plan(&request)
        .await
        .unwrap();
    let approval = CodingAuthorization::scoped(&proposed).unwrap();
    let job = fixture
        .application
        .create_for_start(request, plan, &approval, CancellationToken::new())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap();
    (fixture, provider, job, output)
}

#[tokio::test]
async fn a_start_run_records_ordered_lifecycle_spans() {
    let (fixture, _provider, job, _output) = run_start("lifecycle-spans").await;
    let spans = spans(&fixture, &job);
    println!("lifecycle spans (ms):");
    for span in &spans {
        let started = span["started_ms"].as_u64().unwrap_or(0);
        let ended = span["ended_ms"].as_u64().unwrap_or(0);
        println!(
            "  {:<18} {started:>5}..{ended:<5} = {}",
            span["phase"].as_str().unwrap_or("?"),
            ended.saturating_sub(started)
        );
    }

    for name in [
        "app_open",
        "proposal",
        "approval",
        "context",
        "snapshot",
        "job_setup",
        "run_setup",
        "model_loop",
        "after_loop",
        "settle",
        "candidate_prepare",
        "finalize",
        "pre_model",
        "model",
        "post_model",
    ] {
        phase(&spans, name);
    }

    // The phase spans are one continuous timeline; the derived model
    // boundaries and the elapsed-note summaries are separate lines appended
    // after them.
    let mut previous_end = 0;
    for span in spans.iter().filter(|span| {
        span["summary"] != true
            && !matches!(
                span["phase"].as_str(),
                Some("pre_model" | "model" | "post_model")
            )
    }) {
        let started = span["started_ms"].as_u64().unwrap();
        let ended = span["ended_ms"].as_u64().unwrap();
        assert!(started <= ended, "a span never runs backwards: {span}");
        assert!(
            started >= previous_end,
            "spans are written in lifecycle order: {spans:?}"
        );
        previous_end = ended;
    }

    // The derived model boundaries agree with the phases they bracket.
    assert_eq!(
        phase(&spans, "pre_model")["ended_ms"],
        phase(&spans, "model")["started_ms"]
    );
    assert_eq!(
        phase(&spans, "model")["ended_ms"],
        phase(&spans, "post_model")["started_ms"]
    );
}

/// The per-turn timers land as bounded summaries: one aggregate per phase,
/// and the deferred sandbox probe is measured exactly once per job.
#[tokio::test]
async fn a_start_run_records_bounded_elapsed_summaries() {
    let (fixture, _provider, job, _output) = run_start("lifecycle-summaries").await;
    // The probe is dispatched at toolset composition and runs in the
    // background; a run that never calls `shell` still records it when it
    // finishes. The per-turn timers can land after it under load, so wait for
    // every summary this test reads rather than only the probe.
    let mut recorded = spans(&fixture, &job);
    for _ in 0..60 {
        if ["shell_probe", "message_assembly", "prompt_projection"]
            .iter()
            .all(|phase| recorded.iter().any(|span| span["phase"] == *phase))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        recorded = spans(&fixture, &job);
    }
    let spans = recorded;

    for name in ["message_assembly", "prompt_projection"] {
        let summary = spans
            .iter()
            .find(|span| span["phase"] == name)
            .unwrap_or_else(|| panic!("the run records {name}: {spans:?}"));
        assert_eq!(summary["summary"], true, "{summary}");
        assert!(
            summary["calls"].as_u64().unwrap_or(0) >= 1,
            "{name} was measured: {summary}"
        );
        assert!(summary["total_ms"].is_u64(), "{summary}");
    }

    let probe: Vec<_> = spans
        .iter()
        .filter(|span| span["phase"] == "shell_probe")
        .collect();
    assert_eq!(probe.len(), 1, "one probe per job: {spans:?}");
    assert_eq!(probe[0]["summary"], true, "{probe:?}");
    assert_eq!(probe[0]["calls"], 1, "{probe:?}");

    let summaries = spans.iter().filter(|span| span["summary"] == true).count();
    assert!(
        summaries <= 8 + 8,
        "the elapsed-note summaries are bounded: {summaries}"
    );
}

/// The advisory readiness is computed from the first text token of the final
/// turn and settled before the outcome is written, so the outcome names the
/// same facts without paying the scan after the loop.
#[tokio::test]
async fn a_start_run_precomputes_readiness_before_the_outcome() {
    let (fixture, _provider, job, _output) = run_start("lifecycle-precompute").await;
    assert!(
        job.readiness_precomputed(),
        "the final turn's first text starts the readiness precompute"
    );
    let outcome: Value = serde_json::from_slice(
        &std::fs::read(records(&fixture, &job).join("outcome.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(outcome["readiness"]["w_changed"], true, "{outcome}");
    assert!(
        outcome["changed_paths"]
            .as_array()
            .is_some_and(|paths| paths.iter().any(|path| path["path"] == "turn-one.txt")),
        "the precomputed facts still name the write: {outcome}"
    );
}

/// A start walks the source tree twice — once to plan the baseline the
/// operator approves, once to capture it — and never again. Checkout creation
/// reads the repository identity without the working-tree inventory (the
/// worktree list, whole-tree branch status and index stage entries are not
/// identity), and the create flow's own live checks prove the worktree it
/// made. The same fresh proof then serves the workspace composition and the
/// first readiness view, so neither re-walks W.
#[tokio::test]
async fn a_start_run_does_not_repeat_the_checkout_walk() {
    let (fixture, _provider, job, _output) = run_start("lifecycle-one-pass").await;
    let export = fixture
        .application
        .export(job.job_id(), None, None)
        .await
        .unwrap();
    let kinds: Vec<&str> = export["local_git_processes"]["records"]
        .as_array()
        .expect("the export names the local git receipts")
        .iter()
        .map(|record| record["request_kind"].as_str().unwrap_or("unknown"))
        .collect();
    for (kind, walks) in [
        ("worktree_list", 2),
        ("status_branch", 2),
        ("index_stage_entries", 2),
    ] {
        assert_eq!(
            kinds.iter().filter(|candidate| **candidate == kind).count(),
            walks,
            "one planning walk and one capture walk, no checkout walk: {kinds:?}"
        );
    }
}

#[tokio::test]
async fn a_headless_start_defers_candidate_preparation() {
    let (fixture, _provider, job, output) = run_start("lifecycle-deferred").await;
    let directory = records(&fixture, &job);
    let receipts: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("prepared-candidate-"))
        .collect();
    assert!(
        receipts.is_empty(),
        "a headless start prepares no candidate nobody asked for: {receipts:?}"
    );

    let envelope: Value = serde_json::from_str(&output.answer).expect("the readiness envelope");
    assert_eq!(envelope["candidate_ready"], false, "{envelope}");
    assert!(
        envelope["source_generation_sha256"].is_string(),
        "the view still names the generation: {envelope}"
    );

    // W itself stays reviewable on demand, so apply-back needs no prepared
    // receipt: the root checkout's durable record names the same checkout.
    let record: Value =
        serde_json::from_slice(&std::fs::read(directory.join("root-checkout.json")).unwrap())
            .unwrap();
    let checkout = ManagedCheckoutId::parse(record["checkout_id"].as_str().unwrap()).unwrap();
    job.review_existing(&checkout)
        .await
        .expect("the working checkout is reviewed on demand");
}
