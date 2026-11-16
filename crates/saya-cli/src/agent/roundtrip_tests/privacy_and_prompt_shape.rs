#[tokio::test]
async fn restate_receives_only_sql_and_shape_never_values() {
    // The shape carries a row count and column names by construction — no row
    // values. The text handed to restate must be exactly the SQL and that
    // shape: nothing else, and no cell value from the result the shape
    // describes.
    let sql = "SELECT name, grade FROM students WHERE grade = 'A'";
    let shape = shape(&["name", "grade"], 3);
    // A value that would appear in the result rows but must never reach restate.
    let cell_value = "Alice";
    let restater = ScriptedRestater::new()
        .restate_returns(Some("which students got an A?".into()))
        .judge_returns(None);

    round_trip(
        "names and grades of A students",
        sql,
        &shape,
        &restater,
        &CancellationToken::new(),
    )
    .await;

    let log = restater.log();
    // restate was actually called — inspect what it received.
    let Call::Restate {
        sql: seen_sql,
        shape: seen_shape,
    } = &log[0]
    else {
        panic!("first call must be restate, got {:?}", log[0]);
    };
    // Exactly the SQL — nothing appended, no values concatenated.
    assert_eq!(seen_sql.as_str(), sql);
    // Exactly the shape — row count and column names only.
    assert_eq!(seen_shape.row_count, shape.row_count);
    assert_eq!(seen_shape.columns, shape.columns);
    // A cell value never appears in the SQL or any column name.
    assert!(
        !seen_sql.contains(cell_value),
        "cell value leaked into the SQL"
    );
    assert!(
        seen_shape.columns.iter().all(|c| !c.contains(cell_value)),
        "cell value leaked into the column names"
    );
}

// ===========================================================================
// Prompts — the wording requirements, locked
// ===========================================================================

#[test]
fn restate_prompt_asks_for_a_plain_language_question_and_nothing_else() {
    let p = super::prompts::RESTATE_PROMPT;
    assert!(
        p.contains("plain-language question"),
        "must ask for a plain-language question"
    );
    assert!(
        p.to_lowercase().contains("nothing else"),
        "must demand the question and nothing else"
    );
    assert!(p.contains("hedging"), "must forbid hedging");
    // The question must not be part of the restate prompt — blindness by
    // construction extends to the prompt template.
    assert!(
        !p.contains("{question}"),
        "the restate prompt must not reference the question"
    );
}

#[test]
fn judge_prompt_asks_yes_no_one_sentence_and_judges_substance_not_wording() {
    let p = super::prompts::JUDGE_PROMPT;
    assert!(p.contains("YES or NO"), "must ask for a yes/no");
    assert!(
        p.contains("one") && p.contains("sentence"),
        "must ask for one sentence alongside the yes/no"
    );
    assert!(
        p.contains("counted") && p.contains("filtered") && p.contains("grouped"),
        "must call out counting, filtering, and grouping as the substance that matters"
    );
    assert!(
        p.contains("wording"),
        "must say wording differences do not matter"
    );
}
