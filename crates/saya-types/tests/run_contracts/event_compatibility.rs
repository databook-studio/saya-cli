use super::*;

#[test]
fn legacy_run_started_wire_and_elapsed_clock_event_are_compatible() {
    let legacy: RunEvent = serde_json::from_str(r#"{"type":"run_started"}"#).unwrap();
    assert_eq!(legacy, RunEvent::RunStarted);

    let clock = RunEvent::WallClockObserved {
        origin_unix_ms: 1_000,
        high_water_unix_ms: 1_250,
    };
    let json = serde_json::to_string(&clock).unwrap();
    let decoded: RunEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, clock);
}
