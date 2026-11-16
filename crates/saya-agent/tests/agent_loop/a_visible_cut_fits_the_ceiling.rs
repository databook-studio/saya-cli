// A time-cut that delivered visible text is still a clock, not an output
// window: it never lowers the ceiling later requests run under.

/// The operator sees the visible estimate and the ceiling the run continues
/// under — the configured one, not 85% of the visible tokens.
#[tokio::test]
async fn a_visible_cut_fits_the_measured_window() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CuttingReasoningProvider {
        deltas: CUT_REASONING_DELTAS,
        visible: true,
        requests: requests.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };
    let _ = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await;
    let cuts: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AgentEvent::StreamCutRecovered {
                estimated_tokens,
                ceiling_after,
                ..
            } => Some((*estimated_tokens, *ceiling_after)),
            _ => None,
        })
        .collect();
    assert_eq!(
        cuts.first().copied(),
        Some((CUT_ESTIMATED_TOKENS, 0)),
        "the operator sees the visible estimate and the unlowered ceiling: {cuts:?}"
    );
}

/// One short visible scrap on a ~300 s cut fitted to 1,024 and stuck. A
/// single visible delta must not set that floor.
#[tokio::test]
async fn a_visible_cut_fits_no_floor_from_a_scrap() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CuttingReasoningProvider {
        deltas: 1,
        visible: true,
        requests: requests.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let _ = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: events.clone(),
        },
        CancellationToken::new(),
    )
    .await;
    assert!(
        requests.lock().unwrap().len() > 1,
        "the cut must continue, not stop on the scrap"
    );
    let ceilings: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AgentEvent::StreamCutRecovered { ceiling_after, .. } => Some(*ceiling_after),
            _ => None,
        })
        .collect();
    assert!(
        !ceilings.is_empty() && ceilings.iter().all(|ceiling| *ceiling == 0),
        "a scrap time-cut must not floor later turns to 1024: {ceilings:?}"
    );
}
