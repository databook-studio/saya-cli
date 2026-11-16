#[tokio::test]
async fn restate_is_never_given_the_question() {
    // Blindness is enforced two ways: by construction (the question is not a
    // parameter of `restate`) and at runtime (the question text must not reach
    // the restater through the sql or the shape). A restater that can see the
    // question will echo it and the signal disappears.
    let question = "names of high schoolers who have 3 or more friends";
    let sql = "SELECT s.name FROM high_schoolers s JOIN friendships f \
               ON f.student_id = s.id GROUP BY s.id HAVING COUNT(*) >= 3";
    let shape = shape(&["name"], 2);
    let restater = ScriptedRestater::new()
        .restate_returns(Some("which students have at least three friends?".into()))
        .judge_returns(None);

    round_trip(question, sql, &shape, &restater, &CancellationToken::new()).await;

    let log = restater.log();
    // restate was actually called, so blindness can be checked on a real call.
    let Call::Restate {
        sql: seen_sql,
        shape: seen_shape,
    } = &log[0]
    else {
        panic!("first call must be restate, got {:?}", log[0]);
    };
    assert_eq!(
        seen_sql.as_str(),
        sql,
        "restate must receive the SQL verbatim"
    );
    assert_eq!(seen_shape, &shape);
    // The question text must not appear anywhere restate can see it.
    assert!(
        !seen_sql.contains(question),
        "question leaked into the SQL handed to restate"
    );
    assert!(
        seen_shape.columns.iter().all(|c| !c.contains(question)),
        "question leaked into the column names handed to restate"
    );
}

// ===========================================================================
// 2. Two steps, not one — restate first, then judge
// ===========================================================================

#[tokio::test]
async fn two_steps_restate_then_judge_not_one_call() {
    // A single call "does this SQL answer this question?" would hand the model
    // both the SQL and the question at once, reintroducing the blindness
    // problem the split exists to avoid. Here the two calls are made in order,
    // and judge receives the restatement restate produced (not a fresh look at
    // the SQL).
    let question = "how many orders did each customer place?";
    let sql = "SELECT customer_id, COUNT(*) AS n FROM orders GROUP BY customer_id";
    let restatement = "count of orders per customer".to_string();
    let shape = shape(&["customer_id", "n"], 5);
    let restater = ScriptedRestater::new()
        .restate_returns(Some(restatement.clone()))
        .judge_returns(Some(Verdict {
            agrees: true,
            reason: "both count orders per customer".into(),
        }));

    let rt = round_trip(question, sql, &shape, &restater, &CancellationToken::new()).await;

    let log = restater.log();
    assert_eq!(log.len(), 2, "exactly two model calls: restate, then judge");
    assert!(
        matches!(log[0], Call::Restate { .. }),
        "first call must be restate"
    );
    match &log[1] {
        Call::Judge {
            question: q,
            restatement: r,
        } => {
            assert_eq!(
                q, question,
                "judge must receive the question that was asked"
            );
            assert_eq!(
                r, &restatement,
                "judge must receive the restatement restate produced"
            );
        }
        _ => panic!("second call must be judge, got {:?}", log[1]),
    }
    assert_eq!(rt.restatement.as_deref(), Some(restatement.as_str()));
    assert!(rt.verdict.as_ref().is_some_and(|v| v.agrees));
    assert!(!rt.diverged, "an agreeing verdict is not a divergence");
}

// ===========================================================================
// 3. No restatement → no verdict
// ===========================================================================

#[tokio::test]
async fn no_restatement_means_no_verdict_and_no_divergence() {
    // If restate returns None (the model declined or could not restate), judge
    // is not called and the round-trip reports no divergence.
    let question = "how many orders did each customer place?";
    let sql = "SELECT customer_id, COUNT(*) AS n FROM orders GROUP BY customer_id";
    let shape = shape(&["customer_id", "n"], 5);
    let restater = ScriptedRestater::new()
        .restate_returns(None)
        .judge_returns(Some(Verdict {
            agrees: true,
            reason: "judge must not be reached".into(),
        }));

    let rt = round_trip(question, sql, &shape, &restater, &CancellationToken::new()).await;

    let log = restater.log();
    assert_eq!(log.len(), 1, "only restate ran; judge must not be called");
    assert!(matches!(log[0], Call::Restate { .. }));
    assert!(rt.restatement.is_none());
    assert!(rt.verdict.is_none());
    assert!(!rt.diverged, "no restatement can never be a divergence");
}

// ===========================================================================
// 4. A missing verdict is not a divergence
// ===========================================================================

#[tokio::test]
async fn missing_verdict_is_not_a_divergence() {
    // restate produced a restatement, but judge declined (returned None). The
    // restatement is preserved, but absent evidence is never a disagreement.
    let question = "how many orders did each customer place?";
    let sql = "SELECT customer_id, COUNT(*) AS n FROM orders GROUP BY customer_id";
    let restatement = "count of orders per customer".to_string();
    let shape = shape(&["customer_id", "n"], 5);
    let restater = ScriptedRestater::new()
        .restate_returns(Some(restatement.clone()))
        .judge_returns(None);

    let rt = round_trip(question, sql, &shape, &restater, &CancellationToken::new()).await;

    let log = restater.log();
    assert_eq!(log.len(), 2, "both calls ran");
    assert!(matches!(log[1], Call::Judge { .. }));
    assert_eq!(rt.restatement.as_deref(), Some(restatement.as_str()));
    assert!(rt.verdict.is_none());
    assert!(
        !rt.diverged,
        "a missing verdict is absent evidence, never a disagreement"
    );
}

#[tokio::test]
async fn explicit_disagreement_sets_diverged() {
    // The signal itself: only an explicit disagreeing verdict sets diverged.
    // Both the restatement and the verdict are preserved so a reader can see
    // what differed, not just that something did. This is the real miss from
    // our own run — the question asks for *names of students*, the SQL counts
    // *friendship rows per student*.
    let question = "names of high schoolers who have 3 or more friends";
    let sql = "SELECT s.name FROM high_schoolers s JOIN friendships f \
               ON f.student_id = s.id GROUP BY s.id HAVING COUNT(*) >= 3";
    let restatement =
        "students appearing on either side of a friendship at least 3 times".to_string();
    let verdict = Verdict {
        agrees: false,
        reason: "the SQL counts friendships per student, not students per friendship".into(),
    };
    let restater = ScriptedRestater::new()
        .restate_returns(Some(restatement.clone()))
        .judge_returns(Some(verdict.clone()));

    let rt = round_trip(
        question,
        sql,
        &shape(&["name"], 2),
        &restater,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(rt.restatement.as_deref(), Some(restatement.as_str()));
    assert!(!rt.verdict.as_ref().unwrap().agrees);
    assert_eq!(rt.verdict.as_ref().unwrap().reason, verdict.reason);
    assert!(
        rt.diverged,
        "an explicit disagreement is the only thing that sets diverged"
    );
}
