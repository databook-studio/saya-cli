/// Failed-attempt usage and typed recovery progress are machine events,
/// not unrecognized fallbacks. The successful `Usage` event remains its
/// own variant for the context numerator.
#[test]
fn recovery_events_map_to_dedicated_terminal_variants() {
    let recovery = terminal_event(AgentEvent::provider_recovery(
        saya_agent::ProviderRecoveryPhase::Retrying,
        saya_agent::ProviderRecoveryReason::StreamEnded,
        1,
        3,
    ))
    .expect("recovery progress renders");
    assert!(matches!(
        recovery,
        TerminalEvent::ProviderRecovery {
            phase: saya_agent::ProviderRecoveryPhase::Retrying,
            reason: saya_agent::ProviderRecoveryReason::StreamEnded,
            attempt: 1,
            limit: 3,
        }
    ));
    let usage = terminal_event(AgentEvent::failed_attempt_usage(
        saya_agent::TokenUsage::new(7, 2),
    ))
    .expect("known failed usage renders for machine consumers");
    assert!(matches!(
        usage,
        TerminalEvent::FailedAttemptUsage {
            usage: saya_agent::TokenUsage {
                input_tokens: 7,
                output_tokens: 2,
                ..
            }
        }
    ));
}

/// A stream cut is never silent. The operator sees the cut's own clock, the
/// token estimate taken from the bytes it delivered, and the configured
/// ceiling the run continues under.
#[test]
fn a_stream_cut_reports_its_window_and_fitted_ceiling_in_text_and_ndjson() {
    let cut = AgentEvent::stream_cut_recovered(302, 62_000, 52_700);
    let terminal = terminal_event(cut).expect("a stream cut renders");
    assert!(matches!(
        terminal,
        TerminalEvent::StreamCutRecovered {
            elapsed_secs: 302,
            estimated_tokens: 62_000,
            ceiling_after: 52_700,
        }
    ));
    let text = render_event(&terminal, RenderFormat::Text);
    assert!(
        text.stdout.contains("302s") && text.stdout.contains("52700"),
        "the text line names the window and the fitted ceiling: {text:?}"
    );
    let json = render_event(&terminal, RenderFormat::Ndjson);
    assert!(
        json.stdout.contains(r#""event":"stream_cut_recovered""#)
            && json.stdout.contains(r#""ceiling_after":52700"#),
        "ndjson must tag the cut with its typed fields: {json:?}"
    );
    assert!(
        !json.stdout.contains("not_implemented"),
        "must not fall through to NotImplemented: {json:?}"
    );
}

/// The text adapter closes the open delta line and prints a notice, so a
/// restart mid-answer is legible rather than reading as the model
/// repeating itself; the retried deltas then start on a fresh line.
#[test]
fn turn_reset_closes_the_partial_answer_and_notes_the_retry_in_text() {
    let mut open = false;
    let _ = render_agent(
        AgentEvent::assistant_text("The an"),
        RenderFormat::Text,
        &mut open,
    );
    assert!(open, "the partial answer leaves the delta line open");

    let reset = render_agent(AgentEvent::turn_reset(), RenderFormat::Text, &mut open);
    assert_eq!(
        reset.stdout, "\nprovider stream interrupted — retrying\n",
        "the open line closes and the notice prints on its own line"
    );
    assert!(!open, "the reset closes the delta line");

    // The re-streamed answer starts a fresh line, not appended to the
    // partial text the reset discarded.
    let retried = render_agent(
        AgentEvent::assistant_text("The answer is 42."),
        RenderFormat::Text,
        &mut open,
    );
    assert_eq!(retried.stdout, "The answer is 42.");
    assert!(open);
}

/// The JSON/NDJSON adapter carries the reset under its type tag so a
/// machine consumer can replace accumulated text instead of appending.
#[test]
fn turn_reset_reaches_ndjson_under_its_type_tag() {
    let terminal = terminal_event(AgentEvent::turn_reset()).expect("renders");
    let json = render_event(&terminal, RenderFormat::Ndjson);
    assert!(
        json.stdout.contains(r#""event":"turn_reset""#),
        "ndjson must tag the reset: {json:?}"
    );
    assert!(
        !json.stdout.contains("not_implemented"),
        "must not fall through to NotImplemented: {json:?}"
    );
}
