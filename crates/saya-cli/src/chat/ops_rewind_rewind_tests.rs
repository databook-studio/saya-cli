// `/rewind`: the run's checkpoints newest first, one of them restored, and the
// refusals — an id this run never took, a turn in flight, an empty store.

use super::fixture::{EDITED, FakeRun, case, three_turns, two_turns};
use super::{Refusal, changed_paths, rewind};
use crate::slash::RewindRequest;

#[tokio::test]
async fn rewind_lists_the_checkpoints_newest_first() {
    let (_scratch, folder) = case("rewind-lists", "third\n");
    let run = three_turns(folder);
    let text = rewind(&run, &RewindRequest::List, false)
        .await
        .expect("a listing");
    let newest = text.find("newest words").expect("the newest turn");
    let middle = text.find("middle words").expect("the middle turn");
    let oldest = text.find("oldest words").expect("the oldest turn");
    assert!(
        newest < middle && middle < oldest,
        "the listing runs newest first: {text}"
    );
    for id in ["1", "2", "3"] {
        assert!(
            text.contains(&format!(" {id} ")),
            "every checkpoint's id is listed: {text}"
        );
    }
}

#[tokio::test]
async fn rewind_restores_the_named_checkpoint() {
    let (_scratch, folder) = case("rewind-restore", "second\n");
    let run = two_turns(folder);
    let words = rewind(&run, &RewindRequest::Restore("1".to_owned()), false)
        .await
        .expect("the checkpoint restores");
    assert_eq!(
        run.text(),
        "first\n",
        "the named checkpoint's bytes are back"
    );
    assert_eq!(run.restored(), vec![1]);
    assert!(words.contains(EDITED), "the answer names the file: {words}");
}

#[tokio::test]
async fn rewind_refuses_an_unknown_id_with_the_id() {
    let (_scratch, folder) = case("rewind-unknown", "second\n");
    let run = two_turns(folder);
    let refusal = rewind(&run, &RewindRequest::Restore("9".to_owned()), false)
        .await
        .expect_err("no checkpoint 9");
    assert_eq!(refusal, Refusal::UnknownCheckpoint("9".to_owned()));
    assert!(
        refusal.sentence().contains('9'),
        "the refusal names the id it was given: {}",
        refusal.sentence()
    );
    assert!(run.restored().is_empty());
}

#[tokio::test]
async fn rewind_while_a_turn_runs_is_refused() {
    let (_scratch, folder) = case("rewind-busy", "second\n");
    let run = two_turns(folder);
    for request in [RewindRequest::List, RewindRequest::Restore("1".to_owned())] {
        let refusal = rewind(&run, &request, true)
            .await
            .expect_err("a turn is running");
        assert_eq!(refusal, Refusal::TurnRunning, "{request:?}");
    }
    assert!(run.restored().is_empty());
}

#[tokio::test]
async fn rewind_with_no_checkpoint_says_so() {
    let (_scratch, folder) = case("rewind-empty", "second\n");
    let run = FakeRun::new(folder, Vec::new());
    let refusal = rewind(&run, &RewindRequest::Restore("1".to_owned()), false)
        .await
        .expect_err("no checkpoint");
    assert_eq!(refusal, Refusal::NoCheckpoint);
    assert!(
        !refusal.sentence().is_empty(),
        "the refusal says what to do instead"
    );
}

/// The run's own rewind answers with the files it changed: the ones it wrote
/// back and the ones it removed to match the checkpoint. A key renamed there
/// would otherwise read as "nothing changed".
#[test]
fn a_restore_result_names_every_file_it_changed() {
    let result = serde_json::json!({
        "checkpoint": 2,
        "restored_paths": [{"path": "notes.txt", "sha256": "ab", "size": 3}],
        "removed_paths": ["scratch.md"],
    });
    assert_eq!(
        changed_paths(&result),
        vec!["notes.txt".to_owned(), "scratch.md".to_owned()]
    );
    assert!(
        changed_paths(&serde_json::json!({"checkpoint": 2})).is_empty(),
        "a bare result names no file"
    );
}
