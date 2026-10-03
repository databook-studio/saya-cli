//! The `schema` tool (task Db): live discovery with the state-store cache
//! fallback — the same `agent::state_tools::schema` operation the agent's
//! schema tool uses, over the same connector factory `saya query` uses. The
//! compact tree is the answer the CLI's schema surface renders, wrapped with
//! the profile name; nothing about hosts, paths, or identities is in it.

use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
};

use rmcp::{
    RoleServer,
    model::{CallToolRequestParams, CallToolResponse},
    service::RequestContext,
};
use serde_json::Value;

use super::{connector::build_connector, context::McpContext, policy::ServePolicy, tools};

pub(crate) async fn schema(
    policy: &ServePolicy,
    context: &McpContext,
    request: &CallToolRequestParams,
    request_context: &RequestContext<RoleServer>,
    deadline: tokio::time::Instant,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    if let Some(result) = stopped_before_discovery(request_context.ct.is_cancelled(), deadline) {
        return Ok(result);
    }
    let name = tools::required_string(request, "profile")?;
    let (profile, identity) = match context.allowed_profile(policy.allowlist(), name) {
        Ok(pair) => pair,
        Err(message) => return Ok(tools::error_result(message)),
    };
    let connector = match build_connector(&profile, context).await {
        Ok(connector) => connector,
        Err(message) => return Ok(tools::error_result(message)),
    };
    if let Some(result) = stopped_before_discovery(request_context.ct.is_cancelled(), deadline) {
        return Ok(result);
    }
    if let Err(error) = connector.connect().await {
        return Ok(tools::error_result(error.to_string()));
    }
    if let Some(result) = stopped_before_discovery(request_context.ct.is_cancelled(), deadline) {
        return Ok(result);
    }
    let discovery = crate::agent::state_tools::schema(
        connector.as_ref(),
        Some(&context.store),
        Some(identity.as_str()),
    );
    let compact = match settle_schema_discovery(
        discovery,
        request_context.ct.cancelled(),
        || request_context.ct.is_cancelled(),
        deadline,
    )
    .await
    {
        DiscoveryOutcome::Completed(compact) => compact,
        DiscoveryOutcome::Cancelled => return Ok(tools::error_result("the call was cancelled")),
        DiscoveryOutcome::TimedOut => {
            return Ok(tools::error_result(
                "the database operation exceeded its 30-second cancellation-request deadline",
            ));
        }
    };
    match compact {
        Ok(mut payload) => {
            if let Some(object) = payload.as_object_mut() {
                object.insert("profile".into(), Value::String(name.to_owned()));
            }
            tools::bounded_result(policy, payload)
        }
        Err(error) => Ok(tools::error_result(error.to_string())),
    }
}

enum DiscoveryOutcome<T> {
    Completed(T),
    Cancelled,
    TimedOut,
}

async fn settle_schema_discovery<F, C, S, T>(
    discovery: F,
    source_cancelled: C,
    source_is_cancelled: S,
    deadline: tokio::time::Instant,
) -> DiscoveryOutcome<T>
where
    F: Future<Output = T>,
    C: Future<Output = ()>,
    S: Fn() -> bool,
{
    if source_is_cancelled() {
        return DiscoveryOutcome::Cancelled;
    }
    if tokio::time::Instant::now() >= deadline {
        return DiscoveryOutcome::TimedOut;
    }
    let started = AtomicBool::new(false);
    let discovery = async {
        started.store(true, Ordering::Relaxed);
        discovery.await
    };
    tokio::pin!(discovery);
    tokio::select! {
        biased;
        _ = source_cancelled => {
            if started.load(Ordering::Relaxed) {
                let _discarded = discovery.await;
            }
            DiscoveryOutcome::Cancelled
        }
        _ = tokio::time::sleep_until(deadline) => {
            if started.load(Ordering::Relaxed) {
                let _discarded = discovery.await;
            }
            DiscoveryOutcome::TimedOut
        }
        result = &mut discovery => DiscoveryOutcome::Completed(result),
    }
}

fn stopped_before_discovery(
    source_cancelled: bool,
    deadline: tokio::time::Instant,
) -> Option<CallToolResponse> {
    if source_cancelled {
        Some(tools::error_result("the call was cancelled"))
    } else if tokio::time::Instant::now() >= deadline {
        Some(tools::error_result(
            "the database operation exceeded its 30-second cancellation-request deadline",
        ))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "schema_tool_tests.rs"]
mod schema_tool_tests;
