use super::*;

#[test]
fn the_diff_outcome_texts_are_pinned() {
    assert_eq!(
        render_diff_outcome(&DiffOutcome::Diff {
            text: "diff preview: f.txt".to_owned(),
            truncated: false,
        }),
        (true, "diff preview: f.txt".to_owned())
    );
    assert_eq!(
        render_diff_outcome(&DiffOutcome::NothingToDiff),
        (
            true,
            "No difference: the file already matches its recorded before state.".to_owned()
        )
    );
    assert_eq!(
        render_diff_outcome(&DiffOutcome::BinaryOrNonText {
            before_bytes: 3,
            current_bytes: 5,
        }),
        (
            true,
            "Not diffable: the file is binary or non-text (before 3 B, current 5 B).".to_owned()
        )
    );
    assert_eq!(
        render_diff_outcome(&DiffOutcome::TooLargeToDiff {
            before_bytes: 70_000,
            current_bytes: 10,
        }),
        (
            true,
            "Too large to diff (before 70000 B, current 10 B); the preview is limited \
             to 64 KiB per side."
                .to_owned()
        )
    );
    assert_eq!(
        render_diff_outcome(&DiffOutcome::Refused(RestoreRefusal::NoWorkspace)),
        (
            false,
            "No workspace is bound, so edits cannot be restored.".to_owned()
        )
    );
}
