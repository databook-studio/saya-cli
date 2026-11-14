//! The model-facing rewind end to end: the run's root lists the checkpoints it
//! recorded, restores one path, reads the digest the restore reported, and then
//! restores every path the run changed since that checkpoint. The operator
//! rewind over the same store is `direct_rewind`.

use crate::direct_readiness_facts::support::{Setup, facts_job, run};
use crate::direct_task::{root_checkout, tool_result};

#[tokio::test]
async fn the_model_rewind_lists_restores_one_path_and_removes_the_rest() {
    let (fixture, job, provider) = facts_job(Setup::new(
        "direct-rewind-model",
        114,
        vec!["turn-one.txt".into(), "turn-two.txt".into()],
    ))
    .await;
    run(&job).await;

    // The listing is newest first, bounded, and names each checkpoint's paths.
    let listed = tool_result(&provider, "rewind-list");
    let checkpoints = listed["checkpoints"]
        .as_array()
        .unwrap_or_else(|| panic!("the model listed the checkpoints: {listed}"));
    assert_eq!(checkpoints.len(), 3, "{listed}");
    assert_eq!(checkpoints[0]["id"], "3");
    assert_eq!(checkpoints[2]["id"], "1");
    assert_eq!(listed["maximum_checkpoints"], 32);
    assert!(
        checkpoints[0]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "turn-two.txt"),
        "the newest checkpoint holds both changed paths: {listed}"
    );

    // Restoring one path leaves the other changed path alone.
    let one = tool_result(&provider, "rewind-one");
    assert_eq!(one["restored_paths"][0]["path"], "turn-one.txt", "{one}");
    assert_eq!(one["removed_paths"].as_array().unwrap().len(), 0, "{one}");

    // The digest the restore reported is exactly what a following read
    // observes, so `edit` can use it without another read.
    let read = tool_result(&provider, "read-after-rewind");
    assert!(
        read["content"]
            .as_str()
            .is_some_and(|content| content.ends_with("one\n")),
        "{read}"
    );
    assert_eq!(one["restored_paths"][0]["sha256"], read["digest"], "{one}");
    assert_eq!(one["restored_paths"][0]["size"], read["size"], "{one}");
    assert!(
        tool_result(&provider, "read-turn-two")
            .get("digest")
            .is_some(),
        "the path outside `paths` is still there"
    );

    // Restoring every path removes the file that did not exist at the
    // checkpoint, and the working checkout matches it exactly.
    let all = tool_result(&provider, "rewind-all");
    assert_eq!(all["checkpoint"], 1, "{all}");
    assert!(
        all["removed_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "turn-two.txt"),
        "{all}"
    );
    let checkout = root_checkout(&fixture, &job).await;
    assert_eq!(
        std::fs::read_to_string(checkout.join("turn-one.txt")).unwrap(),
        "one\n"
    );
    assert!(
        matches!(
            std::fs::read(checkout.join("turn-two.txt")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        ),
        "the file created after the checkpoint is gone"
    );
}
