use super::{LifecycleTimeline, MAX_ELAPSED_PHASES, MAX_SPANS, read};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn records_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("saya-lifecycle-{}-{stamp}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn spans_are_written_in_enter_order_after_attach() {
    let timeline = LifecycleTimeline::new();
    timeline.enter("proposal");
    timeline.enter("approval");
    let records = records_dir();
    timeline.attach(&records);
    timeline.enter("snapshot");
    timeline.end();

    let spans = read(&records);
    let phases: Vec<_> = spans
        .iter()
        .filter_map(|span| span["phase"].as_str())
        .collect();
    assert_eq!(phases, ["app_open", "proposal", "approval", "snapshot"]);

    let mut previous_end = 0;
    for span in &spans {
        let started = span["started_ms"].as_u64().unwrap();
        let ended = span["ended_ms"].as_u64().unwrap();
        assert!(started <= ended);
        assert!(started >= previous_end);
        previous_end = ended;
    }
    fs::remove_dir_all(&records).unwrap();
}

/// More distinct phases than the note cap admits, so the test can prove both
/// aggregation and the bound.
const EXTRA_PHASES: [&str; MAX_ELAPSED_PHASES + 4] = [
    "extra_0", "extra_1", "extra_2", "extra_3", "extra_4", "extra_5", "extra_6", "extra_7",
    "extra_8", "extra_9", "extra_10", "extra_11",
];

#[test]
fn elapsed_notes_are_aggregated_and_bounded() {
    let timeline = LifecycleTimeline::new();
    let records = records_dir();
    timeline.attach(&records);
    timeline.note_elapsed("message_assembly", Duration::from_millis(5));
    timeline.note_elapsed("message_assembly", Duration::from_millis(7));
    for phase in EXTRA_PHASES {
        timeline.note_elapsed(phase, Duration::from_millis(1));
    }
    timeline.end();

    let spans = read(&records);
    let assembly: Vec<_> = spans
        .iter()
        .filter(|span| span["phase"] == "message_assembly")
        .collect();
    assert_eq!(assembly.len(), 1, "one summary per phase: {spans:?}");
    assert_eq!(assembly[0]["summary"], true, "{assembly:?}");
    assert_eq!(assembly[0]["calls"], 2, "{assembly:?}");
    assert_eq!(assembly[0]["total_ms"], 12, "{assembly:?}");

    let summaries = spans.iter().filter(|span| span["summary"] == true).count();
    assert_eq!(
        summaries, MAX_ELAPSED_PHASES,
        "the note phases are bounded: {spans:?}"
    );
    fs::remove_dir_all(&records).unwrap();
}

#[test]
fn span_count_is_bounded() {
    let timeline = LifecycleTimeline::new();
    let records = records_dir();
    timeline.attach(&records);
    for _ in 0..(MAX_SPANS + 8) {
        timeline.enter("spin");
    }
    timeline.end();

    assert_eq!(read(&records).len(), MAX_SPANS);
    fs::remove_dir_all(&records).unwrap();
}
