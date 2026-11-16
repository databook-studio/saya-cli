/// The turn ceiling: `SAYA_AGENT_MAX_TURNS=3` stops the loop after exactly
/// three answering turns plus the ceiling salvage call — proving the
/// env-parsed value is what the loop actually received. The tool-call value
/// (7) is distinct from the turn value (3) so a swap is detectable: with the
/// tool-call value on the turn field the loop would consume more scripted
/// rounds before stopping.
#[test]
fn turn_ceiling_stops_the_loop_at_the_planted_turn_count() {
    let (requests, tools, output) = run_scenario(
        "turn-ceiling",
        three_rounds_then_text(CEILING_TEXT),
        &[
            ("SAYA_AGENT_MAX_TURNS", "3"),
            ("SAYA_AGENT_MAX_TOOL_CALLS", "7"),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a ceiling stop salvages a best answer: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        requests, 4,
        "three answering turns plus the ceiling salvage call (tools={tools:?})",
    );
    assert_eq!(
        tools,
        vec![true, true, true, false],
        "the first three requests carry tools, the salvage call strips them",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(CEILING_TEXT),
        "the salvaged answer is the scripted text: {stdout}",
    );
}

/// The tool-call ceiling: `SAYA_AGENT_MAX_TOOL_CALLS=2` stops the loop when
/// a turn's batch would cross it — after two executed calls and one refused
/// batch, with the salvage call. The turn value (9) is distinct from the
/// tool-call value (2) so a swap is detectable: with the turn value on the
/// tool-call field the loop would run nine answering turns instead of three.
#[test]
fn tool_call_ceiling_stops_the_loop_at_the_planted_call_count() {
    let (requests, tools, output) = run_scenario(
        "tool-call-ceiling",
        three_rounds_then_text(CEILING_TEXT),
        &[
            ("SAYA_AGENT_MAX_TURNS", "9"),
            ("SAYA_AGENT_MAX_TOOL_CALLS", "2"),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a ceiling stop salvages a best answer: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        requests, 4,
        "two calls executed, the third turn's batch refused, plus the salvage \
         call (tools={tools:?})",
    );
    assert_eq!(
        tools,
        vec![true, true, true, false],
        "the first three requests carry tools, the salvage call strips them",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(CEILING_TEXT),
        "the salvaged answer is the scripted text: {stdout}",
    );
}

/// The continuation ceiling: `SAYA_AGENT_MAX_CONTINUATIONS=2` re-instructs
/// the model exactly twice on a deterministic truncation, then surfaces the
/// truncation error. The script serves three truncations (the initial call
/// plus two continuations) and then plain text: the planted ceiling binds
/// first, so the run fails without ever reaching the text.
#[test]
fn continuation_ceiling_bounds_the_truncation_retries() {
    let (requests, _tools, output) = run_scenario(
        "continuation-ceiling",
        vec![truncated_body(), truncated_body(), truncated_body()],
        &[("SAYA_AGENT_MAX_CONTINUATIONS", "2")],
    );
    assert_eq!(
        requests, 3,
        "one initial call plus exactly the planted two continuations",
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "the final truncation must surface as a failure",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("truncated"),
        "the truncation error surfaced, not something else: {stderr}",
    );
}
