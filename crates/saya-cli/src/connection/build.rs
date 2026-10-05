use super::registry::{ConnectionEntry, ConnectionRegistry};
use crate::agent::runtime::AgentRuntimeError;
use crate::local_metrics::LocalMetrics;
use futures_util::stream::{self, StreamExt};
use saya_agent::{AgentMetric, AgentMetricOutcome, AgentMetricPhase};
use saya_config::SecretResolver;
use saya_connectors::{ConnectorOptions, build_connector_with_prompt};
use saya_types::DatabaseProfile;
use std::path::Path;
use std::time::Instant;

const MAX_CONCURRENT_SECONDARY_CONNECTIONS: usize = 8;

enum SecondaryResult {
    Success {
        name: String,
        entry: ConnectionEntry,
    },
    Failure {
        name: String,
        reason: String,
    },
}

fn record_connect(metrics: Option<&LocalMetrics>, started: Instant, ok: bool) {
    let Some(metrics) = metrics else { return };
    metrics.record(&AgentMetric::Phase {
        phase: AgentMetricPhase::DatabaseConnect,
        elapsed_ns: u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
        outcome: if ok {
            AgentMetricOutcome::Succeeded
        } else {
            AgentMetricOutcome::Failed
        },
    });
}

/// Builds a registry of live connections: the primary plus each secondary.
/// The primary MUST connect (its failure is returned as an error). Each secondary is
/// connected concurrently (bounded cap of 8) with no interactive auth; any secondary that
/// fails to build or connect is captured as a failure and returned along with the registry.
pub(crate) async fn build_registry(
    (resolver, cache_scope): (&dyn SecretResolver, &Path),
    query_timeout_seconds: u64,
    can_prompt: bool,
    primary_name: &str,
    primary_profile: &DatabaseProfile,
    secondaries: &[(String, DatabaseProfile)],
    metrics: Option<&LocalMetrics>,
) -> Result<(ConnectionRegistry, Vec<(String, String)>), AgentRuntimeError> {
    let mut registry = ConnectionRegistry::new(primary_name);

    let connector = build_connector_with_prompt(
        primary_profile,
        resolver,
        ConnectorOptions {
            query_timeout_seconds,
            ..Default::default()
        },
        can_prompt,
    )
    .await
    .map_err(|err| AgentRuntimeError::Database(err.to_string()))?;

    let started = Instant::now();
    let connected = connector.connect().await;
    record_connect(metrics, started, connected.is_ok());
    connected.map_err(|err| AgentRuntimeError::Database(err.to_string()))?;

    let dialect = connector.dialect();
    let profile_id =
        crate::profile_identity::profile_identity(primary_name, primary_profile, cache_scope);

    registry.insert(
        primary_name,
        ConnectionEntry {
            connector,
            dialect,
            profile_id: Some(profile_id.to_string()),
        },
    );

    let results = stream::iter(secondaries.iter().enumerate().map(
        |(idx, (name, profile))| async move {
            let failed = |reason: String| {
                (
                    idx,
                    SecondaryResult::Failure {
                        name: name.clone(),
                        reason,
                    },
                )
            };
            let connector = match build_connector_with_prompt(
                profile,
                resolver,
                ConnectorOptions {
                    query_timeout_seconds,
                    ..Default::default()
                },
                false,
            )
            .await
            {
                Ok(c) => c,
                Err(err) => {
                    return failed(err.to_string());
                }
            };

            let started = Instant::now();
            let connected = connector.connect().await;
            record_connect(metrics, started, connected.is_ok());
            if let Err(err) = connected {
                return failed(err.to_string());
            }

            let dialect = connector.dialect();
            let profile_id = crate::profile_identity::profile_identity(name, profile, cache_scope);

            (
                idx,
                SecondaryResult::Success {
                    name: name.clone(),
                    entry: ConnectionEntry {
                        connector,
                        dialect,
                        profile_id: Some(profile_id.to_string()),
                    },
                },
            )
        },
    ))
    .buffer_unordered(MAX_CONCURRENT_SECONDARY_CONNECTIONS)
    .collect::<Vec<_>>()
    .await;

    let mut sorted_results = results;
    sorted_results.sort_by_key(|(idx, _)| *idx);

    let mut failures = Vec::new();
    for (_, result) in sorted_results {
        match result {
            SecondaryResult::Success { name, entry } => {
                registry.insert(&name, entry);
            }
            SecondaryResult::Failure { name, reason } => {
                failures.push((name, reason));
            }
        }
    }

    Ok((registry, failures))
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "build_metrics_tests.rs"]
mod metrics_tests;
