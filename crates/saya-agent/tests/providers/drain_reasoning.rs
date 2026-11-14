/// Collects every `ReasoningDelta` a stream emits, plus the `Done` sentinel.
/// the per-provider reasoning tests assert reasoning is
/// captured when present and absent when the wire carries none. This drain
/// keeps the reasoning events the general `drain` drops on the floor.
async fn drain_reasoning(
    stream: &mut saya_agent::ProviderStream,
) -> Vec<saya_agent::ProviderEvent> {
    let mut seen = Vec::new();
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            event @ saya_agent::ProviderEvent::ReasoningDelta(_) => seen.push(event),
            saya_agent::ProviderEvent::Done => {
                seen.push(saya_agent::ProviderEvent::Done);
                break;
            }
            _ => {}
        }
    }
    seen
}
