use super::*;

/// The adapter's download rides the run's shared wallet: a tripped budget
/// is a typed error to the model and the latch the sink reads is set.
#[tokio::test]
async fn a_download_through_the_adapter_trips_the_budget_latch() {
    let (root, workspace) = workspace("download-latch");
    let content = vec![b'a'; 40];
    let (net, _calls) = StaticNet::serve(&content, 4);
    let budget = DownloadBudget::new(10);
    let tools = adapter(net, workspace, budget.clone(), FetchLimits::for_tool_lane());
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({"url": url("/doc"), "destination": "downloads/doc.bin"}),
        )
        .await
        .expect_err("the budget of 10 trips on the third 4-byte chunk");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("download budget of 10 bytes tripped after 8"),
        "typed pause, not overrun: {detail}"
    );
    assert!(budget.tripped(), "the latch the sink reads is set");
    assert!(
        !root.join("downloads/doc.bin").exists(),
        "nothing landed at the destination"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Two concurrent downloads against one small wallet — the loop runs up to
/// four calls per turn — neither writes past the limit, both end typed, and
/// the latch is set: the trip is atomic and total across concurrent
/// callers ("paused, not overrun" is literal).
#[tokio::test]
async fn concurrent_downloads_neither_overrun_nor_miss_the_trip() {
    let (root, workspace) = workspace("concurrent");
    let content = vec![b'z'; 40];
    let (net, _calls) = StaticNet::serve(&content, 4);
    let budget = DownloadBudget::new(24);
    let first = adapter(
        net.clone(),
        Arc::clone(&workspace),
        budget.clone(),
        FetchLimits::for_tool_lane(),
    );
    let second = adapter(net, workspace, budget.clone(), FetchLimits::for_tool_lane());
    let (left, right) = tokio::join!(
        first.execute(
            "http_download",
            serde_json::json!({"url": url("/left"), "destination": "downloads/left.bin"}),
        ),
        second.execute(
            "http_download",
            serde_json::json!({"url": url("/right"), "destination": "downloads/right.bin"}),
        ),
    );
    assert!(budget.tripped(), "at least one refusal must trip the latch");
    for (name, outcome) in [("left", left), ("right", right)] {
        let error = outcome.expect_err("24 bytes cannot serve two 40-byte files");
        let ToolError::Fetch(detail) = error else {
            panic!("typed: {error:?}")
        };
        assert!(
            detail.contains("download budget of 24 bytes tripped"),
            "{name}'s download must stop at its next chunk boundary: {detail}"
        );
    }
    let downloads = root.join("downloads");
    let on_disk: u64 = std::fs::read_dir(&downloads)
        .expect("partials exist")
        .filter_map(|entry| entry.ok())
        .filter(|entry| !entry.file_name().to_string_lossy().ends_with(".json"))
        .map(|entry| entry.metadata().expect("size").len())
        .sum();
    assert!(
        on_disk <= 24,
        "neither download wrote past the wallet: {on_disk} bytes on disk"
    );
    let _ = std::fs::remove_dir_all(root);
}
