#[tokio::test]
async fn execute_then_tally_picks_the_largest_group() {
    // Two runs agree on the same answer; a third disagrees. The vote decides.
    let mut results = HashMap::new();
    results.insert(
        "SELECT a FROM t WHERE id = 1".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    results.insert(
        "SELECT a FROM t WHERE id = 1".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    results.insert(
        "SELECT a FROM t WHERE id = 2".to_string(),
        rows_result(&["a"], vec![json!([2])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT a FROM t WHERE id = 1"),
            nominated("SELECT a FROM t WHERE id = 1"),
            nominated("SELECT a FROM t WHERE id = 2"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert_eq!(decision.votes, 2);
    assert_eq!(decision.margin, 1);
    assert!(!decision.tied);
    assert!(!decision.probe_broke_tie);
    // A None nomination and a failed statement both become result: None and
    // take no part in the vote — covered by the degenerate and probe-fail
    // tests below; here every nomination executed.
}

// ===========================================================================
// 2. Deciding `ordered`
// ===========================================================================

#[tokio::test]
async fn ordered_is_set_when_any_candidate_has_top_level_order_by() {
    // Three candidates, all with a top-level ORDER BY. Two return [1,2,3]; the
    // third returns the same rows reversed under a different ORDER BY. Because
    // order is part of the answer, the reversed result is a *different* answer:
    // groups are {0,1}=2 and {2}=1, so votes is 2. Had `ordered` been false,
    // all three would match unordered and votes would be 3. This proves the
    // ordered comparison was used.
    let mut results = HashMap::new();
    results.insert(
        "SELECT a FROM t ORDER BY a".to_string(),
        rows_result(&["a"], vec![json!([1]), json!([2]), json!([3])]),
    );
    results.insert(
        "SELECT a FROM t ORDER BY a".to_string(),
        rows_result(&["a"], vec![json!([1]), json!([2]), json!([3])]),
    );
    results.insert(
        "SELECT a FROM t ORDER BY a DESC".to_string(),
        rows_result(&["a"], vec![json!([3]), json!([2]), json!([1])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT a FROM t ORDER BY a"),
            nominated("SELECT a FROM t ORDER BY a"),
            nominated("SELECT a FROM t ORDER BY a DESC"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    // votes == 2 is the proof: ordered comparison split the reversed result
    // out of the group rather than merging it.
    assert_eq!(decision.votes, 2);
    assert_eq!(decision.margin, 1);
}

#[tokio::test]
async fn ordered_is_not_set_when_order_by_is_only_in_a_subquery() {
    // ORDER BY buried in a derived table is not top-level, so row order is
    // *not* part of the answer. Two candidates return the same rows in a
    // different order and must therefore agree (one group, a winner) rather
    // than tie.
    let mut results = HashMap::new();
    results.insert(
        "SELECT * FROM (SELECT a FROM t ORDER BY a) sub".to_string(),
        rows_result(&["a"], vec![json!([1]), json!([2])]),
    );
    results.insert(
        "SELECT * FROM (SELECT a FROM t ORDER BY a DESC) sub".to_string(),
        rows_result(&["a"], vec![json!([2]), json!([1])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT * FROM (SELECT a FROM t ORDER BY a) sub"),
            nominated("SELECT * FROM (SELECT a FROM t ORDER BY a DESC) sub"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert_eq!(decision.votes, 2);
    assert!(!decision.tied);
}

#[tokio::test]
async fn ordered_is_not_set_by_order_by_inside_a_string_constant() {
    // The literal text 'order by' in a string constant must not be mistaken
    // for a clause. No real ORDER BY, so order is not part of the answer and
    // the two differently-ordered results agree.
    let mut results = HashMap::new();
    results.insert(
        "SELECT 'order by' AS s, a FROM t".to_string(),
        rows_result(
            &["s", "a"],
            vec![json!(["order by", 1]), json!(["order by", 2])],
        ),
    );
    results.insert(
        "SELECT 'order by' AS s, a FROM t".to_string(),
        rows_result(
            &["s", "a"],
            vec![json!(["order by", 2]), json!(["order by", 1])],
        ),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT 'order by' AS s, a FROM t"),
            nominated("SELECT 'order by' AS s, a FROM t"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert_eq!(decision.votes, 2);
    assert!(!decision.tied);
}

// ===========================================================================
// 3. No tie: the vote decides (and no probe runs)
// ===========================================================================

#[tokio::test]
async fn no_tie_runs_zero_probe_statements() {
    // A clear winner must not cost any fan-out probe. The executor is called
    // exactly once per nomination and never for a COUNT(*) probe.
    let mut results = HashMap::new();
    results.insert(
        "SELECT a FROM t WHERE id = 1".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    results.insert(
        "SELECT a FROM t WHERE id = 1".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    results.insert(
        "SELECT a FROM t WHERE id = 2".to_string(),
        rows_result(&["a"], vec![json!([2])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[
            nominated("SELECT a FROM t WHERE id = 1"),
            nominated("SELECT a FROM t WHERE id = 1"),
            nominated("SELECT a FROM t WHERE id = 2"),
        ],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert!(!decision.tied);
    assert_eq!(decision.fanout, vec![None, None, None]);
    // Exactly three calls — the three nominations. No probe was run.
    assert_eq!(executor.calls().len(), 3);
    assert!(
        !executor.calls().iter().any(|s| s.contains("COUNT(*)")),
        "no probe statement should run when the vote is decisive"
    );
}
