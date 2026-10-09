use super::*;

/// The FIFO is bounded: the 33rd request overflows it — the FIFO is cleared
/// and marked desynchronised rather than silently dropping the oldest
/// candidate, which would pair every later completion one position late.
#[test]
fn pending_candidates_overflow_clears_the_fifo_and_marks_it_desynchronised() {
    let (mut app, mut state) = app_and_state();
    let messages: Vec<StreamMsg> = (1..=33)
        .map(|n| sql_request(&format!("SELECT {n}"), None))
        .collect();
    app.request.stream = Some(stream_with(messages));
    app.drain_stream(&mut state);
    assert!(
        app.pending_queries.is_empty(),
        "the overflow cleared the FIFO instead of dropping the oldest candidate"
    );
    assert!(
        app.pending_queries_desync,
        "the overflow marked the pairing desynchronised"
    );
    assert_eq!(
        selectable_sql(&app),
        None,
        "no request promoted without its own completion"
    );
}

/// F4: once the FIFO overflows, the pairing between requests and completions
/// is off by one — dropping the oldest candidate makes every later completion
/// pop one position late, so a success would promote the *next* request: a
/// query whose own outcome may be the later failure. After the overflow no
/// completion may promote anything for the rest of the turn, and
/// `last_query` keeps its previous value.
#[test]
fn fifo_overflow_never_promotes_a_mispaired_completion() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 0", None),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(selectable_sql(&app), Some("SELECT 0"));

    let mut messages: Vec<StreamMsg> = (1..=33)
        .map(|n| sql_request(&format!("SELECT {n}"), None))
        .collect();
    // The completions arrive in request order: the first succeeds, the
    // second (request #2) fails. Under drop-oldest the first success pops
    // request #2 and promotes a failed query's SQL — the mispairing.
    messages.push(completed("1 row"));
    messages.push(completed("read-only database tool failed"));
    app.request.stream = Some(stream_with(messages));
    app.drain_stream(&mut state);
    assert!(
        app.pending_queries_desync,
        "the overflow marked the pairing desynchronised"
    );
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 0"),
        "after the overflow no completion promotes anything: last_query is unchanged"
    );
}

/// F4: the overflow's desynchronised state is per-turn: the overflow clears
/// the FIFO (rather than silently dropping the oldest candidate), and the
/// next turn pairs normally again once the overflow's turn has ended.
#[test]
fn overflow_flag_resets_next_turn() {
    let (mut app, mut state) = app_and_state();
    let messages: Vec<StreamMsg> = (1..=33)
        .map(|n| sql_request(&format!("SELECT {n}"), None))
        .collect();
    app.request.stream = Some(stream_with(messages));
    app.drain_stream(&mut state);
    assert!(
        app.pending_queries.is_empty(),
        "the overflow cleared the FIFO rather than dropping the oldest candidate"
    );
    assert!(
        app.pending_queries_desync,
        "the overflow marked the pairing desynchronised for the rest of the turn"
    );
    assert_eq!(selectable_sql(&app), None);

    // The turn ends: the desynchronised mark resets with the FIFO.
    app.request.stream = Some(stream_with(vec![StreamMsg::Done(Ok(AgentOutput {
        answer: "the answer".into(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage: TokenUsage::new(0, 0),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }))]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert!(
        !app.pending_queries_desync,
        "the flag reset when the turn ended"
    );

    // The next turn pairs normally again.
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 1"),
        "normal pairing resumes after the overflow's turn ended"
    );
}

/// R4-9: a TurnReset also clears a desynchronised mark the discarded attempt
/// left (its FIFO overflowed): the retry queues and pairs again from a
/// trusted, empty FIFO instead of staying muted for the rest of the turn.
#[test]
fn a_turn_reset_clears_a_desynchronised_mark_so_the_retry_pairs_again() {
    let (mut app, mut state) = app_and_state();
    let mut messages: Vec<StreamMsg> = (1..=33)
        .map(|n| sql_request(&format!("SELECT {n}"), Some("analytics")))
        .collect();
    // The overflow desynchronised the discarded attempt...
    // ...and the turn resets to retry it.
    messages.push(StreamMsg::Event(AgentEvent::turn_reset()));
    messages.push(sql_request("SELECT 40", Some("analytics")));
    messages.push(completed("1 row"));
    app.request.stream = Some(stream_with(messages));
    app.drain_stream(&mut state);
    assert!(
        !app.pending_queries_desync,
        "the reset cleared the discarded attempt's desynchronised mark"
    );
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 40"),
        "the retry's request queued and its completion promoted it"
    );
}
