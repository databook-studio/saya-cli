async fn drain(stream: &mut saya_agent::ProviderStream) -> Vec<saya_agent::ProviderEvent> {
    let mut seen = Vec::new();
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            event @ saya_agent::ProviderEvent::Usage(_) => seen.push(event),
            saya_agent::ProviderEvent::Done => {
                seen.push(saya_agent::ProviderEvent::Done);
                break;
            }
            _ => {}
        }
    }
    seen
}
