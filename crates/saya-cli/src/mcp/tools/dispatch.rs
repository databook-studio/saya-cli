/// Run one tool body. The name dispatch is the single extension point; an
/// unknown name is a method error, per the MCP spec. A row-returning tool
/// called while data sharing is off is refused as a tool-level error with
/// the gate's own words (invariant 2). Every complete response passes the
/// dispatch gate before it leaves.
pub(crate) async fn run(
    policy: &ServePolicy,
    context: &McpContext,
    request: &CallToolRequestParams,
    request_context: &RequestContext<RoleServer>,
    request_deadline: tokio::time::Instant,
) -> Result<CallToolResponse, ErrorData> {
    let response = if catalog::is_row_returning(&request.name) && !policy.allow_data_sharing() {
        Ok(error_result(format!(
            "{} is not available: data sharing is off for this server; restart \
             with --allow-data-sharing or set [ai] allow_data_sharing",
            request.name
        )))
    } else {
        match request.name.as_ref() {
            "contracts" => context_tools::contracts(policy, context, request).await,
            "investigation_run" => {
                replay_tools::investigation_run(
                    policy,
                    context,
                    request,
                    request_context,
                    request_deadline,
                )
                .await
            }
            "list_profiles" => run_list_profiles(policy, request).await,
            "query" => {
                query_tool::query(policy, context, request, request_context, request_deadline).await
            }
            "schema" => {
                schema_tool::schema(policy, context, request, request_context, request_deadline)
                    .await
            }
            other => Err(ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool: {other}"),
                None,
            )),
        }
    };
    // The reply line carries the JSON-RPC envelope with the client's actual
    // request id; the dispatch gate measures it exactly before anything
    // leaves. A protocol error keeps its own (short) envelope.
    response.map(|response| bounded_response(policy, response, &request_context.id))
}

async fn run_list_profiles(
    policy: &ServePolicy,
    request: &CallToolRequestParams,
) -> Result<CallToolResponse, ErrorData> {
    if let Some(arguments) = request.arguments.as_ref()
        && !arguments.is_empty()
    {
        return Err(ErrorData::invalid_params(
            "list_profiles takes no arguments",
            None,
        ));
    }
    bounded_result(policy, profiles_payload(policy))
}

/// The `list_profiles` answer: the startup allowlist, names and dialects
/// only — the same set `tools/list` advertises, and the only set a client
/// can ever reach.
pub(crate) fn profiles_payload(policy: &ServePolicy) -> serde_json::Value {
    json!({
        "profiles": policy
            .allowlist()
            .iter()
            .map(|profile| json!({"name": profile.name, "dialect": profile.dialect}))
            .collect::<Vec<_>>(),
    })
}
