/// Asserts `open_browser` neither executed nor completed, and was denied with
/// a non-empty reason — the observable shape of "the policy gated this call".
fn assert_open_browser_gated(calls: &[String], events: &[AgentEvent]) {
    assert!(
        !calls.iter().any(|name| name == "open_browser"),
        "the gated tool must not execute; got calls {calls:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCompleted { name, .. } if name == "open_browser"
        )),
        "the gated tool must not complete; got events {events:?}"
    );
    let denied = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolDenied { name, reason } if name == "open_browser" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .expect("the gated tool must surface a ToolDenied event with a reason");
    assert!(
        !denied.is_empty(),
        "the denial must carry a clear reason, not be a silent skip"
    );
}
