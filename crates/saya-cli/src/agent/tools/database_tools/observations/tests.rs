use super::*;

fn obs(tool: &str) -> ToolObservation {
    ToolObservation {
        tool: tool.into(),
        outcome: ObservationOutcome::Succeeded,
        profile: None,
        objects: Vec::new(),
        columns: Vec::new(),
        row_count: None,
        truncated: None,
        references_partial: false,
    }
}

fn obs_with_objects(tool: &str, objects: &[&[&str]]) -> ToolObservation {
    ToolObservation {
        objects: objects
            .iter()
            .map(|parts| parts.iter().map(|s| s.to_string()).collect())
            .collect(),
        ..obs(tool)
    }
}

#[test]
fn record_then_drain_returns_in_order_and_empties() {
    let log = ObservationLog::new();
    log.record(obs("a"));
    log.record(obs("b"));
    let drained = log.drain();
    assert_eq!(
        drained
            .observations
            .iter()
            .map(|o| &o.tool)
            .collect::<Vec<_>>(),
        &["a", "b"]
    );
    assert!(!drained.truncated);
    // A second drain is empty: a turn cannot double-count.
    let again = log.drain();
    assert!(again.observations.is_empty());
    assert!(!again.truncated);
}

#[test]
fn cap_holds_and_drain_reports_truncation() {
    let log = ObservationLog::new();
    for i in 0..40 {
        log.record(obs(&format!("t{i}")));
    }
    let drained = log.drain();
    assert_eq!(drained.observations.len(), MAX_OBSERVATIONS);
    assert_eq!(drained.observations.last().unwrap().tool, "t31");
    assert!(drained.truncated);
}

#[test]
fn touched_matches_underspecified_observations_anchored_at_trailing_parts() {
    let log = ObservationLog::new();
    // A 3-part observation pins its catalog: the same 3-part proposal matches,
    // but a different-catalog proposal for the same schema+object does not —
    // the query touched `analytics.public.orders`, never `cat.public.orders`.
    log.record(obs_with_objects(
        "bounded_sql_query",
        &[&["analytics", "public", "orders"]],
    ));
    assert!(log.touched("analytics", "public", "orders"));
    assert!(!log.touched("cat", "public", "orders"));

    // An under-specified observation (fewer parts) matches any proposal that
    // agrees on the trailing parts the observation actually named.
    let log = ObservationLog::new();
    log.record(obs_with_objects(
        "bounded_sql_query",
        &[&["public", "orders"]],
    ));
    assert!(log.touched("analytics", "public", "orders"));
    assert!(log.touched("cat", "public", "orders"));

    let log = ObservationLog::new();
    log.record(obs_with_objects("bounded_sql_query", &[&["orders"]]));
    assert!(log.touched("analytics", "public", "orders"));
    assert!(log.touched("cat", "schema", "orders"));
}

#[test]
fn touched_misses_a_different_object() {
    let log = ObservationLog::new();
    log.record(obs_with_objects(
        "bounded_sql_query",
        &[&["public", "orders"]],
    ));
    assert!(!log.touched("analytics", "public", "lineitems"));
    assert!(!log.touched("analytics", "staging", "orders"));
}

#[test]
fn touched_ignores_failed_and_denied_observations() {
    let log = ObservationLog::new();
    let mut failed = obs_with_objects("bounded_sql_query", &[&["public", "orders"]]);
    failed.outcome = ObservationOutcome::Failed;
    log.record(failed);
    let mut denied = obs_with_objects("bounded_sql_query", &[&["public", "orders"]]);
    denied.outcome = ObservationOutcome::Denied;
    log.record(denied);
    assert!(
        !log.touched("analytics", "public", "orders"),
        "a failed or denied query touched nothing a proposal can lean on"
    );
}

#[test]
fn touched_is_non_consuming() {
    let log = ObservationLog::new();
    log.record(obs_with_objects(
        "bounded_sql_query",
        &[&["public", "orders"]],
    ));
    assert!(log.touched("cat", "public", "orders"));
    // The log is unchanged: a second read still sees the observation, and a
    // later drain returns it. `contract_propose` reads mid-turn.
    assert!(log.touched("cat", "public", "orders"));
    assert_eq!(log.drain().observations.len(), 1);
}
