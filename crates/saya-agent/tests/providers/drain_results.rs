/// Collects every stream item including the terminal error, so tests can
/// assert the exact ordering of events around a stream failure.
async fn drain_results(
    stream: &mut saya_agent::ProviderStream,
) -> Vec<Result<saya_agent::ProviderEvent, saya_agent::ProviderError>> {
    let mut seen = Vec::new();
    while let Some(event) = stream.next().await {
        let terminal = event.is_err();
        seen.push(event);
        if terminal {
            break;
        }
    }
    seen
}
