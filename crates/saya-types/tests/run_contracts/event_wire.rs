use super::*;

// ---------------------------------------------------------------------------
// RunEvent: serde-tagged, one line, and usage absence is "unknown", not zero
// ---------------------------------------------------------------------------

#[test]
fn usage_event_serializes_absence_as_null_never_zero() {
    let event = RunEvent::Usage {
        endpoint: "deepseek".to_string(),
        tokens: None,
        turns: None,
        tool_calls: None,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(
        json.contains("\"tokens\":null"),
        "absence must stay null: {json}"
    );
    assert!(
        json.contains("\"turns\":null"),
        "absence must stay null: {json}"
    );
    assert!(
        json.contains("\"cached_input_tokens\":null"),
        "an unreported cache figure must stay null: {json}"
    );
    assert!(
        json.contains("\"cache_creation_input_tokens\":null"),
        "an unreported cache-write figure must stay null: {json}"
    );
    assert!(!json.contains("\"tokens\":0"));
    assert!(!json.contains("\"cached_input_tokens\":0"));

    let reported = RunEvent::Usage {
        endpoint: "deepseek".to_string(),
        tokens: Some(0),
        turns: Some(3),
        tool_calls: Some(7),
        cached_input_tokens: Some(0),
        cache_creation_input_tokens: Some(0),
    };
    let json = serde_json::to_string(&reported).unwrap();
    assert!(
        json.contains("\"tokens\":0"),
        "a reported zero stays 0: {json}"
    );
    assert!(
        json.contains("\"cached_input_tokens\":0"),
        "a reported cache zero stays 0, never unknown: {json}"
    );

    let back: RunEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(reported, back);

    // A journal line written before the cache fields existed still parses:
    // the fields default to "unreported", never to a reported zero.
    let legacy = r#"{"type":"usage","endpoint":"deepseek","tokens":120,"turns":1,"tool_calls":2}"#;
    let old: RunEvent = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        old,
        RunEvent::Usage {
            endpoint: "deepseek".to_string(),
            tokens: Some(120),
            turns: Some(1),
            tool_calls: Some(2),
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        },
        "an old journal line must read as unreported cache figures, not zeros"
    );
}

#[test]
fn plan_approved_journal_line_without_scopes_replays_as_scopes_unstated() {
    // A journal written before the `scopes` payload existed carries no
    // `scopes` field; `#[serde(default)]` keeps it parseable as `None` —
    // "scopes unstated here" — so old runs resume unchanged, on the
    // documented spec fallback. (Moved assertion, U8: the field became
    // `Option<Vec<String>>` so this line can no longer fold into the
    // stated-empty case below; the pre-field meaning it pins is the same.)
    let legacy = r#"{"type":"plan_approved"}"#;
    let old: RunEvent = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        old,
        RunEvent::PlanApproved { scopes: None },
        "an old PlanApproved line must replay as scopes unstated, not fail to parse"
    );

    // And the round trip of a modern line keeps the tokens verbatim, in
    // declaration order — the journal is the authority a resume re-grants
    // from, so the field must survive the round trip exactly.
    let approved = RunEvent::PlanApproved {
        scopes: Some(vec!["interpreter:python3".to_string()]),
    };
    let json = serde_json::to_string(&approved).unwrap();
    let back: RunEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(approved, back);
    assert!(
        json.contains(r#""scopes":["interpreter:python3"]"#),
        "the journal line carries the approved scopes verbatim: {json}"
    );
}

/// The two ways a `PlanApproved` line can state no scopes mean different
/// things, and the field keeps them apart (U8): a `--allow none` run writes
/// the explicit empty payload — `"scopes":[]` — which replays as the stated
/// empty approval (`Some(vec![])`, the resume re-grants nothing from it),
/// while the pre-field line stays `None`. The bytes distinguish them on
/// disk; the type must not fold them back together, or a `spec.json`
/// edited between invocations could widen a none run's resume.
#[test]
fn a_none_run_s_journal_line_parses_as_the_stated_empty_approval() {
    let none_run = r#"{"type":"plan_approved","scopes":[]}"#;
    let stated: RunEvent = serde_json::from_str(none_run).unwrap();
    assert_eq!(
        stated,
        RunEvent::PlanApproved {
            scopes: Some(vec![])
        },
        "an empty payload is the stated empty approval — a `--allow none` run — \
         never the pre-field fallback shape"
    );

    // The stated empty approval round-trips as the exact bytes drive
    // writes for a none run: the payload is present and empty.
    let json = serde_json::to_string(&stated).unwrap();
    let back: RunEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(stated, back);
    assert!(
        json.contains("\"scopes\":[]"),
        "the none run's journal line carries the empty payload: {json}"
    );

    // And `None` never reaches the wire as `null`: a line a pre-payload
    // saya could parse must remain what older readers can parse.
    let legacy = serde_json::to_string(&RunEvent::PlanApproved { scopes: None }).unwrap();
    assert_eq!(
        legacy, r#"{"type":"plan_approved"}"#,
        "the unstated shape serializes field-less, the old line's own bytes"
    );
}

#[test]
fn every_event_variant_round_trips_through_serde() {
    let events = vec![
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::PlanApproved {
            scopes: Some(vec![]),
        },
        RunEvent::PlanApproved {
            scopes: Some(vec![
                "fetch:https+example.com".to_string(),
                "interpreter:python3".to_string(),
            ]),
        },
        RunEvent::StepStarted { step: 0 },
        RunEvent::StepCompleted { step: 0 },
        RunEvent::StepFailed { step: 0 },
        RunEvent::Deliverables {
            step: 0,
            entries: vec![
                Deliverable::present("report.md", 29, "4634".to_string()),
                Deliverable::missing("draft.md"),
            ],
        },
        RunEvent::Paused {
            reason: PauseReason::BudgetExhausted,
        },
        RunEvent::Paused {
            reason: PauseReason::WallClockExceeded,
        },
        RunEvent::Completed,
        RunEvent::Failed {
            code: RunFailureCode::SafetyQuery,
        },
        RunEvent::Failed {
            code: RunFailureCode::Provider,
        },
        RunEvent::Failed {
            code: RunFailureCode::ConnectionConfig,
        },
        RunEvent::Cancelled,
        RunEvent::Usage {
            endpoint: "deepseek".to_string(),
            tokens: Some(120),
            turns: Some(1),
            tool_calls: Some(2),
            cached_input_tokens: Some(60),
            cache_creation_input_tokens: None,
        },
        RunEvent::WallClockObserved {
            origin_unix_ms: 1_000,
            high_water_unix_ms: 1_000,
        },
        RunEvent::DownloadedBytes { bytes: 0 },
        RunEvent::DownloadedBytes { bytes: 97 },
    ];
    for event in events {
        let json = serde_json::to_string(&event).unwrap();
        let back: RunEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back, "round trip failed for {json}");
    }
}
