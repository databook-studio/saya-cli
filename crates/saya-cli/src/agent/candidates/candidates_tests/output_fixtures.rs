fn rows_result(columns: &[&str], rows: Vec<Value>) -> QueryResult {
    QueryResult {
        columns: columns.iter().map(|s| s.to_string()).collect(),
        rows,
        row_count: 0,
        truncated: false,
        executed_sql: String::new(),
    }
}

/// An `AgentOutput` whose `answer_sql` is `Some(sql)` and whose `answer` names
/// it, so a test can tell the winning attempt's prose from the rest.
fn output_with_sql(sql: &str) -> AgentOutput {
    AgentOutput {
        answer: format!("answer for {sql}"),
        events: Vec::new(),
        used_bounded_sql_query: true,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: Some(sql.to_string()),
    }
}

/// An `AgentOutput` whose turn settled on a clarification: the loop emitted
/// `ClarificationNeeded` and completed, so the returned Ok output carrying the
/// event is a settled pause — the way `loop_runner/clarification.rs` builds it.
fn clarification_output(sql: &str) -> AgentOutput {
    let mut out = output_with_sql(sql);
    out.events.push(AgentEvent::clarification_needed(
        "which definition of \"active users\" should I use?",
        vec![
            "sessions in the last 30 days".into(),
            "purchases in the last 90 days".into(),
        ],
    ));
    out
}

/// An `AgentOutput` that designated nothing (`answer_sql: None`).
fn output_without_sql(label: &str) -> AgentOutput {
    AgentOutput {
        answer: format!("answer {label}"),
        events: Vec::new(),
        used_bounded_sql_query: true,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

fn output_with_usage(sql: &str, input: u64, output: u64) -> AgentOutput {
    let mut result = output_with_sql(sql);
    result.usage = TokenUsage::new(input, output);
    result
}

/// The single `ConsensusDecided` event in `captured`, if any.
fn one_consensus(captured: &[AgentEvent]) -> Option<&AgentEvent> {
    captured
        .iter()
        .find(|e| matches!(e, AgentEvent::ConsensusDecided { .. }))
}

fn consensus_count(captured: &[AgentEvent]) -> usize {
    captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::ConsensusDecided { .. }))
        .count()
}

fn assert_consensus(
    event: &AgentEvent,
    sql: Option<&str>,
    attempts: usize,
    voted: usize,
    votes: usize,
    tied: bool,
) {
    let AgentEvent::ConsensusDecided {
        sql: got_sql,
        attempts: got_attempts,
        voted: got_voted,
        votes: got_votes,
        margin: _,
        tied: got_tied,
        probe_broke_tie: _,
    } = event
    else {
        panic!("expected ConsensusDecided, got {event:?}");
    };
    assert_eq!(*got_attempts, attempts, "attempts: {event:?}");
    assert_eq!(*got_voted, voted, "voted: {event:?}");
    assert_eq!(*got_votes, votes, "votes: {event:?}");
    assert_eq!(*got_tied, tied, "tied: {event:?}");
    match (got_sql.as_deref(), sql) {
        (Some(got), Some(want)) => assert_eq!(got, want, "winning SQL mismatch: {event:?}"),
        (None, None) => {}
        _ => panic!("winning SQL mismatch: {event:?} (expected {sql:?})"),
    }
}
