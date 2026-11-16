use super::*;

// ---------------------------------------------------------------------------
// RunId
// ---------------------------------------------------------------------------

#[test]
fn run_id_accepts_identifier_shape() {
    let id = RunId::parse("run-2026-09-10_a1").unwrap();
    assert_eq!(id.as_str(), "run-2026-09-10_a1");
}

#[test]
fn run_id_rejects_empty_oversize_or_fancy_characters() {
    assert!(matches!(
        RunId::parse(""),
        Err(RunContractError::InvalidRunId)
    ));
    assert!(matches!(
        RunId::parse(&"a".repeat(129)),
        Err(RunContractError::InvalidRunId)
    ));
    assert!(matches!(
        RunId::parse("run abc"),
        Err(RunContractError::InvalidRunId)
    ));
    assert!(matches!(
        RunId::parse("run/../etc"),
        Err(RunContractError::InvalidRunId)
    ));
}

#[test]
fn run_id_round_trips_through_serde() {
    let id = run_id();
    let json = serde_json::to_string(&id).unwrap();
    let back: RunId = serde_json::from_str(&json).unwrap();
    assert_eq!(id, back);
}

#[test]
fn run_id_serde_rejects_invalid_shape() {
    let result: Result<RunId, _> = serde_json::from_str("\"run abc\"");
    assert!(result.is_err());
}
