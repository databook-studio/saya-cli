/// The token counts one provider call reported reach the NDJSON stream
/// under their own type tag, named by which call they describe so a
/// consumer can keep the answer's cost apart from the extraction call's.
#[test]
fn usage_reaches_ndjson_under_its_type_tag_named_by_call() {
    let event = AgentEvent::usage(
        saya_agent::UsageCall::Extraction,
        saya_agent::TokenUsage::new(40, 10),
    );
    let terminal = terminal_event(event).expect("usage renders headlessly");
    assert!(
        !matches!(terminal, TerminalEvent::NotImplemented { .. }),
        "Usage must not fall through to the catch-all: {terminal:?}"
    );
    let json = render_event(&terminal, RenderFormat::Ndjson);
    assert!(
        json.stdout.contains(r#""event":"usage""#),
        "ndjson must tag the usage event: {json:?}"
    );
    assert!(
        json.stdout.contains(r#""call":"extraction""#),
        "the call kind must name the extraction call: {json:?}"
    );
    assert!(
        json.stdout.contains(r#""input_tokens":40"#)
            && json.stdout.contains(r#""output_tokens":10"#),
        "the counts must be carried: {json:?}"
    );

    let answering = render_event(
        &terminal_event(AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            saya_agent::TokenUsage::new(3, 7),
        ))
        .expect("renders"),
        RenderFormat::Ndjson,
    );
    assert!(
        answering.stdout.contains(r#""call":"answer""#),
        "the answering call is named too: {answering:?}"
    );
}

/// A reported zero and an unreported number must not collapse on the wire.
/// `cached_input_tokens` is `Option` precisely so "the provider says zero
/// cached tokens" stays a claim it made, and "the provider said nothing"
/// stays `null` — a cache hit rate computed over the two has to be able to
/// tell "0%" from "unknown".
#[test]
fn reported_zero_and_unreported_cached_tokens_serialize_distinctly() {
    let reported_zero = render_event(
        &terminal_event(AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            saya_agent::TokenUsage::new(10, 5).with_cached_input(Some(0)),
        ))
        .expect("renders"),
        RenderFormat::Ndjson,
    );
    assert!(
        reported_zero.stdout.contains(r#""cached_input_tokens":0"#),
        "a reported zero must serialize as a number: {:?}",
        reported_zero.stdout
    );
    let unreported = render_event(
        &terminal_event(AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            saya_agent::TokenUsage::new(10, 5),
        ))
        .expect("renders"),
        RenderFormat::Ndjson,
    );
    assert!(
        unreported.stdout.contains(r#""cached_input_tokens":null"#),
        "an unreported figure must serialize as null: {:?}",
        unreported.stdout
    );
    assert_ne!(
        reported_zero.stdout, unreported.stdout,
        "the two must be distinguishable on the wire"
    );
}

/// The text adapter renders nothing for a usage event: the interactive
/// surfaces for it already exist (the per-turn token line and `/usage`),
/// and it must not fall through to `Not implemented: unrecognized agent
/// event` under a correct answer.
#[test]
fn usage_renders_to_nothing_in_the_text_adapter() {
    let event = AgentEvent::usage(
        saya_agent::UsageCall::Answer,
        saya_agent::TokenUsage::new(3, 7),
    );
    let rendered = render_agent(event, RenderFormat::Text, &mut false);
    assert_eq!(rendered.stdout, "", "the text adapter stays silent");
    assert_eq!(rendered.stderr, "", "nothing on stderr either");
}

/// A usage event carries no content, so a text consumer that ignores it
/// sees byte-identical output whether or not it arrives. It renders to
/// nothing itself, and it closes an open delta line the way any other
/// non-assistant event does — it arrives after the answer's text finished
/// streaming, where closing is what `ToolRequested` already does. The
/// property pinned is the invariant on the stream's shape: interleaving
/// usage into a run changes nothing a text reader sees.
#[test]
fn interleaving_usage_leaves_the_text_output_unchanged() {
    let mut open = false;
    let answer = render_agent(
        AgentEvent::assistant_text("thinking"),
        RenderFormat::Text,
        &mut open,
    );
    let closed = render_agent(AgentEvent::complete(), RenderFormat::Text, &mut open);
    let without_usage = format!("{}{}", answer.stdout, closed.stdout);

    let mut open = false;
    let answer = render_agent(
        AgentEvent::assistant_text("thinking"),
        RenderFormat::Text,
        &mut open,
    );
    let usage = render_agent(
        AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            saya_agent::TokenUsage::new(3, 7),
        ),
        RenderFormat::Text,
        &mut open,
    );
    let closed = render_agent(AgentEvent::complete(), RenderFormat::Text, &mut open);
    let with_usage = format!("{}{}{}", answer.stdout, usage.stdout, closed.stdout);

    assert_eq!(
        with_usage, without_usage,
        "usage must not change the text stream"
    );
    // And the delta line was still closed, not left hanging open.
    assert!(!open, "the line must be closed after Complete");
}

/// The JSON adapter carries the usage event under its type tag rather than
/// the NotImplemented fallback (every adapter that renders events).
#[test]
fn json_adapter_carries_the_usage_event_under_its_type_tag() {
    let event = AgentEvent::usage(
        saya_agent::UsageCall::Answer,
        saya_agent::TokenUsage::new(3, 7).with_cached_input(Some(2)),
    );
    let te = terminal_event(event).expect("this event renders headlessly");
    let rendered = render_event(&te, RenderFormat::Json);
    assert!(
        rendered.stdout.contains(r#""event":"usage""#),
        "{:?}",
        rendered.stdout
    );
    assert!(
        !rendered.stdout.contains("not_implemented"),
        "{:?}",
        rendered.stdout
    );
}

/// `TurnReset` maps to a real TerminalEvent variant (not the
/// NotImplemented catch-all) — a retry signal the renderer understands
/// must never print `unrecognized agent event`.
#[test]
fn turn_reset_renders_as_its_own_variant_not_the_catch_all() {
    let terminal = terminal_event(AgentEvent::turn_reset()).expect("turn reset renders");
    assert!(
        matches!(terminal, TerminalEvent::TurnReset),
        "must map to the dedicated variant: {terminal:?}"
    );
}
