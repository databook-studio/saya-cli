// `/undo`: the files the last turn edited, put back from the checkpoint taken
// before it — and the three cases in which it refuses and says why.

use super::fixture::{EDITED, FakeRun, Recorded, case, two_turns};
use super::{Refusal, TURN_RUNNING, undo};

#[tokio::test]
async fn undo_restores_the_files_the_last_turn_edited_and_names_them() {
    let (_scratch, folder) = case("undo-restores", "second\n");
    let run = two_turns(folder);
    let words = undo(&run, false).await.expect("undo restores");
    assert_eq!(
        run.text(),
        "first\n",
        "the file holds what it held before the last turn edited it"
    );
    assert_eq!(
        run.restored(),
        vec![1],
        "the checkpoint taken before the last turn is the one restored"
    );
    assert!(
        words.contains(EDITED),
        "the answer names every file it restored: {words}"
    );
}

#[tokio::test]
async fn undo_names_a_file_another_run_changed_since_and_leaves_it() {
    let (_scratch, folder) = case("undo-changed", "second\n");
    let run = two_turns(folder).with_changed_elsewhere(&[EDITED]);
    let words = undo(&run, false).await.expect("undo runs");
    assert!(
        words.contains(&format!("changed since by another run: {EDITED}")),
        "the skipped file is named as another run's change: {words}"
    );
    assert_eq!(
        run.text(),
        "second\n",
        "and it is never overwritten: the other run's bytes stand"
    );
}

/// The same rule for `/rewind <id>`, which reaches the same restore.
#[tokio::test]
async fn rewind_names_a_file_another_run_changed_since() {
    let (_scratch, folder) = case("rewind-changed", "second\n");
    let run = two_turns(folder).with_changed_elsewhere(&[EDITED]);
    let words = super::rewind(
        &run,
        &crate::slash::RewindRequest::Restore("1".into()),
        false,
    )
    .await
    .expect("the rewind runs");
    assert!(
        words.contains(&format!("changed since by another run: {EDITED}")),
        "the skipped file is named as another run's change: {words}"
    );
}

#[tokio::test]
async fn undo_with_no_edited_turn_says_so() {
    let (_scratch, folder) = case("undo-empty", "second\n");
    let run = FakeRun::new(folder, Vec::new());
    let refusal = undo(&run, false).await.expect_err("nothing to undo");
    assert_eq!(refusal, Refusal::NothingToUndo);
    assert_eq!(
        refusal.sentence(),
        "no turn of this run has edited a file yet, so there is nothing to undo"
    );
    assert_eq!(run.text(), "second\n", "a refusal changes no file");
}

#[tokio::test]
async fn undo_needs_a_checkpoint_before_the_last_turn() {
    let (_scratch, folder) = case("undo-one", "second\n");
    let run = FakeRun::new(
        folder,
        vec![Recorded {
            turn: 1,
            words: Some("the only turn"),
            bytes: "second\n",
        }],
    );
    let refusal = undo(&run, false).await.expect_err("no earlier checkpoint");
    assert_eq!(refusal, Refusal::NoEarlierCheckpoint { turn: 1 });
    let words = refusal.sentence();
    assert!(
        words.contains("baseline"),
        "the answer names the remedy: {words}"
    );
    assert!(run.restored().is_empty(), "nothing was restored");
}

#[tokio::test]
async fn undo_while_a_turn_runs_is_refused() {
    let (_scratch, folder) = case("undo-busy", "second\n");
    let run = two_turns(folder);
    let refusal = undo(&run, true).await.expect_err("a turn is running");
    assert_eq!(refusal, Refusal::TurnRunning);
    assert_eq!(
        refusal.sentence(),
        "a turn is running: /undo and /rewind restore files, so they wait until it ends"
    );
    assert_eq!(TURN_RUNNING, refusal.sentence());
    assert!(
        run.restored().is_empty(),
        "a refused undo touches no file: the turn in flight owns them"
    );
    assert_eq!(run.text(), "second\n");
}
