//! A coding surface's MCP lane: the tools of the servers the user configured
//! in `[mcp_servers]`, under the same consent rule as the surface's shell.
//!
//! It follows the shell's and the web lane's placement, and for the same
//! reason: a tool that reaches outside the workspace comes with the working
//! checkout. A job with no write path composes the coordinator's delegation
//! surface alone, which reaches no server.
//!
//! The root holds one lane for the whole run. A helper composes its own when
//! the job composes it, under the helper's own caller line: the job layer
//! does not pool server connections across surfaces, so each helper's lane is
//! its own connection, started when the helper is composed and closed when it
//! ends. Servers start lazily — a job whose mode reaches no server, or whose
//! configuration names none, composes `None` and starts nothing.

use std::sync::Arc;

use saya_types::AgentId;

use crate::agent::tools::consent::Caller;
use crate::commands::coding::NativeCodingJob;
use crate::mcp::client::McpTools;

/// Builds the root's MCP lane, or `None` where this job's mode reaches no
/// server or the configuration names none.
pub(crate) async fn build(job: &NativeCodingJob) -> Option<Arc<McpTools>> {
    connect(
        job,
        Caller::Root {
            job_id: job.job_id().clone(),
        },
    )
    .await
}

/// Builds one helper's MCP lane under the job's mode and operator, with the
/// helper's own caller line. `None` under the same conditions as [`build`].
pub(crate) async fn build_for_helper(
    job: &NativeCodingJob,
    agent_id: AgentId,
) -> Option<Arc<McpTools>> {
    connect(
        job,
        Caller::Helper {
            agent_id,
            job_id: job.job_id().clone(),
        },
    )
    .await
}

/// One lane over the job's configured servers. The same resolver every other
/// egressing seam uses: the secret values saya already holds, so an entry's
/// `env` reference resolves the way the rest of the configuration does.
async fn connect(job: &NativeCodingJob, caller: Caller) -> Option<Arc<McpTools>> {
    let servers = job.mcp_servers().clone();
    if servers.is_empty() {
        return None;
    }
    let resolver: Arc<dyn saya_config::SecretResolver> =
        Arc::new(job.runtime_config().secret_resolver());
    McpTools::connect_async(
        servers,
        resolver,
        job.approval_policy(),
        job.operator(),
        caller,
        job.cancellation(),
    )
    .await
}
