// A surface whose tool list can grow re-states it, and the loop re-reads that
// statement before every request: the coding root advertises the two calls that
// reach a background run only once this job has started one. The list only ever
// grows — the first request's tools are the second's prefix — so the cached
// prefix changes once and never again.

use std::sync::atomic::{AtomicBool, Ordering};

/// The three definitions every request starts with, plus one the surface adds
/// after its first call.
struct GrowingTools {
    grown: AtomicBool,
}

#[async_trait]
impl ToolExecutor for GrowingTools {
    fn advertised_definitions(&self) -> Option<Vec<ToolDefinition>> {
        let mut list = definitions();
        if self.grown.load(Ordering::Acquire) {
            list.push(ToolDefinition {
                name: "shell_output".into(),
                description: "read a background run".into(),
                read_only: true,
                parameters: serde_json::json!({"type":"object"}),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: saya_agent::LocalStateEffect::None,
                },
                concurrency: ToolConcurrency::Serial,
                completion: None,
            });
        }
        Some(list)
    }

    async fn execute(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.grown.store(true, Ordering::Release);
        Ok(serde_json::json!({"tables": []}))
    }
}

/// The same shape, but the surface never re-states its list: the loop keeps the
/// definitions the run was constructed with, however the executor changes.
struct StaticTools;

#[async_trait]
impl ToolExecutor for StaticTools {
    async fn execute(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({"tables": []}))
    }
}

fn names(tools: &[ToolDefinition]) -> Vec<&str> {
    tools.iter().map(|tool| tool.name.as_str()).collect()
}

#[tokio::test]
async fn a_growing_tool_list_reaches_the_next_request() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let tools = GrowingTools {
        grown: AtomicBool::new(false),
    };
    let output = run_agent(
        &ToolThenAnswerProvider {
            requests: requests.clone(),
        },
        &tools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the run completes");
    assert_eq!(output.answer, "done");

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2, "one tool turn and one answer");
    assert_eq!(
        names(&captured[0].tools),
        vec!["bounded_sql_query", "bounded_sql_query_all", "schema_discovery"],
        "the first request carries the list the run was constructed with"
    );
    assert_eq!(
        names(&captured[1].tools),
        vec![
            "bounded_sql_query",
            "bounded_sql_query_all",
            "schema_discovery",
            "shell_output"
        ],
        "the second request carries what the surface re-stated"
    );
    assert_eq!(
        captured[0].tools.len() + 1,
        captured[1].tools.len(),
        "the list grows by exactly the tool the surface added"
    );
    assert_eq!(
        serde_json::to_vec(&captured[0].tools).unwrap(),
        serde_json::to_vec(&captured[1].tools[..captured[0].tools.len()]).unwrap(),
        "the first request's tools stay the second's prefix, so the cached prefix changes once"
    );
}

#[tokio::test]
async fn a_surface_that_states_nothing_keeps_the_list_it_was_given() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent(
        &ToolThenAnswerProvider {
            requests: requests.clone(),
        },
        &StaticTools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the run completes");
    assert_eq!(output.answer, "done");

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2);
    for request in captured.iter() {
        assert_eq!(
            names(&request.tools),
            vec!["bounded_sql_query", "bounded_sql_query_all", "schema_discovery"],
            "the default keeps the list the run was constructed with"
        );
    }
}
