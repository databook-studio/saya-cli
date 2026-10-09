//! E2-3: a re-run entry (`--refresh`, `/chart`, bare `/explain`) must
//! refuse when the stored alias now resolves to a different source — the old
//! SQL must not silently run against the wrong database. Snapshot and report
//! read the capture only and keep working, printing the old provenance.

use super::Dispatch;
use crate::interactive::tui::transcript::BlockKind;
use saya_types::SqlDialect;
use saya_types::{EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs};

#[path = "query_source_tests/support.rs"]
mod support;
use support::{GuardFixture, STARTED_UNIX_MS, current_identity, sample_result};

/// Control: with nothing rebound, `--refresh` re-runs normally.
#[test]
fn refresh_reruns_when_source_unchanged() {
    let mut fx = GuardFixture::build("control");
    fx.with_capture(Some(current_identity(&fx.runtime)));
    let path = fx.root.join("refresh.csv");
    let outcome = fx.dispatch(&format!("/export --refresh {}", path.display()));
    assert!(
        matches!(outcome, Dispatch::SqlTask(_)),
        "an unchanged source re-runs the last query"
    );
    assert!(
        fx.error_text()
            .is_none_or(|text| !text.contains("Refused to re-run")),
        "no refusal is said on the control path"
    );
}

/// MISMATCH: after the alias is rebound, each re-run entry is refused and
/// spawns no SQL task.
#[test]
fn rerun_entries_refuse_after_alias_rebind() {
    for tag in ["refresh", "chart", "explain"] {
        let mut fx = GuardFixture::build(tag);
        fx.with_capture(Some(current_identity(&fx.runtime)));
        fx.rebind_demo();
        let line = match tag {
            "refresh" => format!(
                "/export --refresh {}",
                fx.root.join("refresh.csv").display()
            ),
            "chart" => format!("/chart line {}", fx.root.join("chart.png").display()),
            _ => "/explain".to_string(),
        };
        let outcome = fx.dispatch(&line);
        assert!(
            matches!(outcome, Dispatch::Handled),
            "{tag}: a mismatched re-run is handled inline, spawning nothing"
        );
        let text = fx.error_text().expect("{tag}: the refusal is said");
        assert!(
            text.contains("Refused to re-run"),
            "{tag}: the refusal names itself: {text}"
        );
        assert!(
            text.contains("different source"),
            "{tag}: the refusal names the cause: {text}"
        );
        assert!(
            !text.contains("sentinel-alice"),
            "{tag}: no row content leaks"
        );
    }
}

/// Snapshot and report never re-run: after the rebind they still read the
/// OLD capture, labelled with its original provenance.
#[test]
fn snapshot_and_report_keep_reading_the_old_capture() {
    let mut fx = GuardFixture::build("snapshot-report");
    fx.with_capture(Some(current_identity(&fx.runtime)));
    let short_id = fx
        .captured
        .as_ref()
        .expect("capture")
        .evidence
        .short_id()
        .to_owned();
    fx.rebind_demo();

    let snap = fx.root.join("snapshot.csv");
    let outcome = fx.dispatch(&format!("/export --snapshot {}", snap.display()));
    assert!(
        matches!(outcome, Dispatch::Handled),
        "snapshot stays inline"
    );
    let written = std::fs::read_to_string(&snap).expect("snapshot written");
    assert!(
        written.contains("sentinel-alice"),
        "rows come from the capture"
    );
    let said = fx
        .transcript
        .blocks()
        .iter()
        .rev()
        .find(|block| block.kind == BlockKind::System)
        .map(|block| block.text.clone())
        .expect("snapshot success is said");
    assert!(
        said.contains(&short_id),
        "the old execution is named: {said}"
    );

    let report = fx.root.join("report.md");
    let outcome = fx.dispatch(&format!("/report --rows 2 {}", report.display()));
    assert!(matches!(outcome, Dispatch::Handled), "report stays inline");
    let written = std::fs::read_to_string(&report).expect("report written");
    assert!(
        written.contains("sentinel-alice"),
        "rows come from the capture"
    );
    assert!(
        written.contains("Connection label: demo"),
        "old provenance kept"
    );
}

/// Unknown stays unknown: evidence without an identity re-runs as before.
#[test]
fn unknown_identity_reruns_as_before() {
    let mut fx = GuardFixture::build("unknown");
    fx.with_capture(None);
    fx.rebind_demo();
    let path = fx.root.join("refresh.csv");
    let outcome = fx.dispatch(&format!("/export --refresh {}", path.display()));
    assert!(
        matches!(outcome, Dispatch::SqlTask(_)),
        "without a stored identity there is nothing to compare, so the re-run proceeds"
    );
}

/// The pure guard's unknown cases: no capture, no identity, a different
/// label, or an unresolvable label all mean "no proven mismatch".
#[test]
fn pure_guard_unknown_stays_unknown() {
    let fx = GuardFixture::build("pure");
    let stored = current_identity(&fx.runtime);
    let evidence = |label: &str, identity: Option<String>| {
        ExecutionEvidence::for_result(
            &sample_result(),
            ExecutionEvidenceArgs {
                execution_id: "xabc-1".to_string(),
                connection_label: label.to_string(),
                connection_identity: identity,
                dialect: SqlDialect::Sqlite,
                max_rows: 100,
                started_unix_ms: STARTED_UNIX_MS,
                finished_unix_ms: STARTED_UNIX_MS + 2_000,
                source: EvidenceSource::DirectSql,
            },
        )
    };
    assert!(
        super::query_source_guard::rerun_source_mismatch(None, "demo", &fx.runtime).is_none(),
        "no capture, no refusal"
    );
    assert!(
        super::query_source_guard::rerun_source_mismatch(
            Some(&evidence("demo", None)),
            "demo",
            &fx.runtime
        )
        .is_none(),
        "no stored identity, no refusal"
    );
    assert!(
        super::query_source_guard::rerun_source_mismatch(
            Some(&evidence("demo", Some(stored.clone()))),
            "other",
            &fx.runtime
        )
        .is_none(),
        "a different label is not this re-run's business"
    );
    assert!(
        super::query_source_guard::rerun_source_mismatch(
            Some(&evidence("ghost", Some(stored))),
            "ghost",
            &fx.runtime
        )
        .is_none(),
        "an unresolvable label keeps the existing error path"
    );
}
