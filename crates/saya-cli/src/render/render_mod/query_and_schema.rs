fn query_text(result: &QueryResult) -> String {
    let mut output = result.columns.join("\t");
    if !output.is_empty() {
        output.push('\n');
    }
    for row in &result.rows {
        let value = match row {
            serde_json::Value::Array(values) => values
                .iter()
                .map(display_value)
                .collect::<Vec<_>>()
                .join("\t"),
            value => display_value(value),
        };
        output.push_str(&value);
        output.push('\n');
    }
    if result.truncated {
        output.push_str("[truncated]\n");
    }
    output
}

fn display_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        value => value.to_string(),
    }
}

/// Shapes the `ConsensusDecided` line: how many attempts ran, how many voted,
/// and — when there is a winner — which SQL is believed. The line trails the
/// answer, naming the picked query so a reader knows which attempt's prose to
/// trust. No result rows; SQL text only, like `AnswerDesignated`.
fn consensus_text(
    sql: Option<&str>,
    attempts: usize,
    voted: usize,
    votes: usize,
    tied: bool,
    probe_broke_tie: bool,
) -> String {
    let head = format!("consensus · {attempts} attempts, {voted} voted, {votes} agreed");
    match sql {
        Some(sql) => {
            let how = if probe_broke_tie {
                "tie broken by evidence"
            } else {
                "agreed"
            };
            format!("{head} · {how}, believing: {sql}\n")
        }
        None if tied => format!("{head} · tied, no winner\n"),
        None => format!("{head} · no winner\n"),
    }
}

fn schema_text(schema: &SchemaTree) -> String {
    schema
        .databases
        .iter()
        .flat_map(|database| {
            database.schemas.iter().flat_map(move |schema| {
                schema
                    .tables
                    .iter()
                    .map(move |table| format!("{}.{}.{}", database.name, schema.name, table.name))
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}
