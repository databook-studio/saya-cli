/// `render_chart` writes a file and opens a browser, so it must not declare
/// `read_only: true` — the flag feeds the completion summary, and a chart
/// completion reading "read-only database tool completed" would be a false
/// statement. It states its own completion instead. The external-effect
/// declaration is pinned too: nobody may "fix" the advertisement rule later
/// by weakening it.
#[test]
fn render_chart_declares_an_honest_effect() {
    let tools = DatabaseTools::definitions(true, false, false, false, true);
    let chart = tools
        .iter()
        .find(|tool| tool.name == "render_chart")
        .expect("render_chart must be registered when query data is allowed");
    assert!(
        !chart.read_only,
        "render_chart writes a file and spawns a browser; it is not read-only"
    );
    assert!(
        chart.effect.external_side_effect,
        "render_chart launches a process outside saya; the external effect must stay declared"
    );
    assert_eq!(
        chart.completion.as_deref(),
        Some("chart written and opened"),
        "render_chart states what it did instead of a generic label"
    );
}

/// The chart is hidden where no external effect can be allowed: with the
/// permit off, `render_chart` is absent — while every other query-data tool
/// stays advertised, so the hiding is the chart's own gate, not the privacy
/// gate closing.
#[test]
fn render_chart_is_hidden_without_the_external_effect_permit() {
    let hidden: Vec<String> = DatabaseTools::definitions(true, false, false, false, false)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(
        !hidden.iter().any(|name| name == "render_chart"),
        "no external-effect permit: render_chart must be hidden, got: {hidden:?}"
    );
    for name in [
        "bounded_sql_query",
        "bounded_sql_query_all",
        "result_shape",
        "column_health",
        "join_check",
        "designate_answer",
    ] {
        assert!(
            hidden.iter().any(|tool| tool == name),
            "{name} stays advertised while only the chart hides: {hidden:?}"
        );
    }
    let shown: Vec<String> = DatabaseTools::definitions(true, false, false, false, true)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(
        shown.iter().any(|name| name == "render_chart"),
        "with the permit: render_chart is advertised, got: {shown:?}"
    );
}

/// `designate_answer` nominates the statement that answered the question, not
/// an exploratory probe. Two questions nominated nothing at all and one
/// nominated a probe in the benchmark; the description must say plainly that a
/// probe is never the answering query.
#[test]
fn designate_answer_description_forbids_an_exploratory_probe() {
    let tools = DatabaseTools::definitions(true, false, false, false, true);
    let designate = tools
        .iter()
        .find(|tool| tool.name == "designate_answer")
        .expect("designate_answer must be registered when query data is allowed");
    let description = &designate.description;
    assert!(
        description.contains("never an exploratory probe"),
        "the description must plainly forbid nominating a probe: {description}"
    );
    assert!(
        description.contains("the statement that produced the answer"),
        "the description must name the answering statement: {description}"
    );
}
