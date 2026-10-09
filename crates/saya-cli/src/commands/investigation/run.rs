//! `saya investigation run <id>` (S7, D4): replays a saved investigation
//! through the one query path — connector build, `connect`, the safety gate
//! inside `execute`, the same `QueryResult` renderer as `saya query` — with
//! an explicit local target and a review binding that goes stale when the
//! definition revision, target identity, or referenced schema changes. No
//! AI provider is constructed anywhere on this path, and the binding is
//! written only after a successful execution. The operation returns a typed
//! [`RunOutcome`] (C3/D12) so adapters capture the replay instead of
//! re-parsing rendered output.

mod connect;
mod execute;
mod prepare;
mod support;

use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_agent::CancellationToken;
use saya_store::{InvestigationRepository, SqliteStateStore};
use saya_types::investigation::InvestigationId;
use std::time::Instant;

/// What the dispatcher parsed for one replay. The report fields (S12b)
/// carry an optional `--report` destination with its `--rows` opt-in and
/// `--overwrite` decision; `params` carries the raw `--param name=value`
/// bindings (B1f). `machine_driven` marks the MCP replay adapter's runs:
/// such a run can never write or complete review authority.
pub(super) struct RunRequest<'a> {
    pub id: &'a str,
    pub connection: Option<&'a str>,
    pub revalidate: bool,
    pub report: Option<&'a std::path::Path>,
    pub rows: Option<usize>,
    pub overwrite: bool,
    pub params: Vec<String>,
    pub machine_driven: bool,
    pub cancellation: Option<&'a CancellationToken>,
}

pub(super) struct InitialReplay<'a> {
    profile: &'a saya_types::DatabaseProfile,
    id: InvestigationId,
    definition: saya_types::investigation::InvestigationDefinitionV1,
    bound: Vec<saya_types::BoundParam>,
    binding: Option<saya_store::LocalBinding>,
    review: super::run_binding::Review,
    started: Instant,
}

pub(super) struct ConnectedReplay {
    id: InvestigationId,
    definition: saya_types::investigation::InvestigationDefinitionV1,
    bound: Vec<saya_types::BoundParam>,
    binding: Option<saya_store::LocalBinding>,
    review: super::run_binding::Review,
    connector: Box<dyn saya_connectors::DatabaseConnector>,
    request_query: saya_types::QueryRequest,
    started: Instant,
    started_unix_ms: i64,
}

enum Stage<T> {
    Continue(T),
    Finished(Box<super::run_outcome::RunOutcome>),
}

pub(super) async fn run(
    repo: &InvestigationRepository,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
    request: RunRequest<'_>,
) -> Result<super::run_outcome::RunOutcome, Box<dyn std::error::Error>> {
    let initial = match prepare::prepare(repo, runtime, format, &request).await? {
        Stage::Continue(initial) => initial,
        Stage::Finished(outcome) => return Ok(*outcome),
    };
    let connected =
        match connect::connect(runtime, format, can_prompt, state_db, &request, initial).await? {
            Stage::Continue(connected) => connected,
            Stage::Finished(outcome) => return Ok(*outcome),
        };
    execute::execute(repo, runtime, format, state_db, request, connected).await
}
