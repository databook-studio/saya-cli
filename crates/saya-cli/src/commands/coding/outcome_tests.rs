use super::{InPlaceResult, bounded, in_place_facts};
use saya_types::JobId;
use serde_json::json;

#[test]
fn in_place_result_notice() {
    let job = JobId::parse("job-notice").unwrap();
    let paths = vec!["report.md".to_owned()];
    let facts = in_place_facts(
        &InPlaceResult {
            root_path: "/tmp/folder",
            root_id: "loc-abc",
            snapshot_id: "job-notice",
            root_git: false,
            recorded_files: 3,
            recorded_bytes: 40,
            recorded_overflowed: false,
            changed_paths: &paths,
            paths_truncated: false,
            finalized: false,
            finalize_source: None,
            checkpoints: 2,
        },
        &job,
    );
    assert_eq!(facts["mode"], "in_place");
    assert_eq!(facts["root"]["path"], json!("/tmp/folder"));
    assert_eq!(facts["root"]["git"], json!(false));
    assert_eq!(facts["baseline"]["kind"], "edit_time");
    assert_eq!(facts["recorded"]["files"], json!(3));
    assert_eq!(facts["recorded"]["bytes"], json!(40));
    assert_eq!(facts["recorded"]["overflowed"], json!(false));
    assert_eq!(facts["changes"]["files"], json!(1));
    assert_eq!(facts["changes"]["paths"], json!(["report.md"]));
    assert_eq!(facts["review"]["finalized"], json!(false));
    assert_eq!(facts["undo"]["checkpoints"], json!(2));
    assert_eq!(facts["undo"]["baseline_restore"], json!(true));
    let notice = facts["notice"].as_str().expect("a changed job notices");
    assert!(notice.contains("/tmp/folder"), "{notice}");
    assert!(notice.contains("job-notice"), "{notice}");
    assert!(notice.contains("--baseline"), "{notice}");
    assert!(notice.contains("concurrent edit is reported"), "{notice}");
    assert!(notice.contains("File content only"), "{notice}");

    let quiet = in_place_facts(
        &InPlaceResult {
            root_path: "/tmp/folder",
            root_id: "loc-abc",
            snapshot_id: "job-notice",
            root_git: false,
            recorded_files: 3,
            recorded_bytes: 40,
            recorded_overflowed: false,
            changed_paths: &[],
            paths_truncated: false,
            finalized: false,
            finalize_source: None,
            checkpoints: 0,
        },
        &job,
    );
    assert!(quiet.get("notice").is_none(), "{quiet}");
    assert_eq!(
        quiet["changes"],
        json!({"files":0,"paths":[],"paths_truncated":false})
    );
}

#[test]
fn a_short_text_is_kept_and_not_marked_truncated() {
    assert_eq!(bounded("the answer", 64), ("the answer".to_owned(), false));
}

#[test]
fn a_long_text_is_cut_on_a_character_boundary_and_marked() {
    let text = "é".repeat(40);
    let (bounded_text, truncated) = bounded(&text, 21);
    assert!(truncated);
    assert!(bounded_text.ends_with("[truncated at 21 bytes]"));
    assert!(
        bounded_text.starts_with("éééééééééé"),
        "the cut never splits a character: {bounded_text}"
    );
}
