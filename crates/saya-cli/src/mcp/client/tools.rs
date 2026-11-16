//! One surface holds one MCP client: the configured servers it started, the
//! tools they offered, and the consent rule every call to one of them passes
//! through.
//!
//! # Why a call is consented like a host shell run
//!
//! An MCP server is a program the user configured and a tool call is a
//! request to it. What the server does with that request is the server's
//! business — it is not bounded by a workspace root, a schema, or a read-only
//! SQL gate the way every tool saya composes for itself is. The honest
//! statement of that is the `shell`'s own rule: `bypass` runs it, `ask` puts
//! each call to the operator, and a headless `ask` refuses with the flag that
//! would reach it named. `read-only` and `never` reach no server at all, so
//! the client is not composed under them and no `mcp__` tool is advertised.
//!
//! # What a call's answer is
//!
//! The server's text, bounded and redacted exactly as a `shell` call's output
//! is ([`saya_harness::runner::capture`]): both ends of the ring, the elision
//! marker between them, and the credential values the server was handed in its
//! environment scrubbed out before the model reads a byte. The `is_error` flag
//! is the server's own statement about the call, carried rather than
//! interpreted.
//!
//! # What lives where
//!
//! This file is the client itself — the started servers, the mode, the consent,
//! the advertisement, and the one table that says whether a call may run. The
//! two things that follow from it are beside it: [`call`] is one approved
//! call, and [`approval`] is the consent rule and the definition a decider is
//! asked about, built in one place so the advertisement and the question
//! cannot drift.

use std::sync::Arc;
use std::time::Instant;

use saya_agent::{ApprovalDecider, ApprovalPolicy, CancellationToken, ToolDefinition};
use saya_config::{ResolvedMcpServers, SecretResolver};

use super::bounds::MAX_CONNECT_TOTAL;
use super::name;
use super::server::McpServer;
use crate::agent::tools::consent::{Caller, Operator};

use approval::definition;

mod approval;
mod call;

/// The refusal a call earns under `ask` when no operator is present to answer
/// it. The flag that reaches an MCP server from such a driver is named, so the
/// refusal is actionable rather than merely true.
pub(crate) const MCP_APPROVAL_UNAVAILABLE: &str = "calling a tool on an MCP server needs the \
     user's approval and no operator is available here: a job or session calls MCP tools \
     with --approval-mode bypass";

/// The refusal a call earns under `read-only`. A server is a program with no
/// bounds saya can state, so a mode that promises nothing leaves the machine
/// has no honest way to run one.
pub(crate) const READ_ONLY_REFUSAL: &str = "this session is read-only: no MCP tool is called, because a server is a program whose \
     effects this mode cannot bound";

/// The refusal a call earns under `never`. No client is composed in that mode,
/// so a built tool never reaches this; it is here so the decision stays total.
pub(crate) const NEVER_REFUSAL: &str = "this session calls no MCP tool";

/// Whether a mode advertises MCP tools at all. The two modes that may reach a
/// server are the two that advertise them: `bypass`, which runs a call without
/// asking, and `ask`, which puts each call to the operator — or, where there is
/// no operator, refuses with the reason above.
pub(crate) fn advertised(policy: ApprovalPolicy) -> bool {
    matches!(policy, ApprovalPolicy::Ask | ApprovalPolicy::Bypass)
}

/// Who a call to an MCP tool is put to.
///
/// The coding root has an [`Operator`] — a human, addressed by a subject line
/// naming the job. The interactive session has its per-turn
/// [`ApprovalDecider`], which decides against a tool's own definition. Both
/// answer the same question, so both are carried here rather than one being
/// reconstructed from a subject line.
pub(crate) enum Consent {
    /// Nobody: under `ask` every call is refused with the reason naming the
    /// flag that would reach a server from such a driver.
    Absent,
    /// A human, addressed by the tool's own prompt line.
    Operator(Arc<dyn Operator>),
    /// A surface's own decider, addressed with the tool's own definition.
    Decider(Arc<dyn ApprovalDecider>),
}

/// The MCP client one surface holds.
///
/// Two halves, and the split is what lets one session share one set of
/// servers across every turn: the started servers and their reports are the
/// session's or the job's, and the mode, the consent, the caller, and the
/// cancellation are one *turn's*. [`McpTools::with_consent`] is the seam
/// between them, so a surface that composes per turn (the interactive
/// session) starts each server once and still puts every call to whoever is
/// there when it is made.
pub(crate) struct McpTools {
    servers: Arc<Vec<Arc<McpServer>>>,
    /// One line per server that could not be started, per tool that was
    /// dropped, and per listing that was cut short. Reported, never silent: a
    /// server that is unusable must not look like a server with nothing to
    /// offer.
    reports: Arc<Vec<String>>,
    policy: ApprovalPolicy,
    consent: Consent,
    caller: Caller,
    cancellation: CancellationToken,
}

impl McpTools {
    /// Connects every configured server, bounded, and returns the client.
    ///
    /// `None` means this surface has no MCP tools and no client: the mode reaches
    /// no server, or the configuration names none. A server that fails to start
    /// does not make the client `None` — it is reported, and its tools are simply
    /// absent, which is what "a failing server is reported and its tools are
    /// simply absent" means in practice.
    pub(crate) fn connect(
        servers: &ResolvedMcpServers,
        resolver: &dyn SecretResolver,
        policy: ApprovalPolicy,
        operator: Option<Arc<dyn Operator>>,
        caller: &Caller,
        cancellation: CancellationToken,
    ) -> Option<Arc<Self>> {
        if !advertised(policy) || servers.is_empty() {
            return None;
        }
        let mut reports = Vec::new();
        let mut connected: Vec<Arc<McpServer>> = Vec::new();
        let deadline = Instant::now() + MAX_CONNECT_TOTAL;
        for server in &servers.servers {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                reports.push(format!(
                    "mcp server {:?}: the {}-second limit for starting configured servers \
                     passed before this one was tried",
                    server.name(),
                    MAX_CONNECT_TOTAL.as_secs()
                ));
                continue;
            }
            let env = match resolve_env(server, resolver) {
                Ok(env) => env,
                Err(reason) => {
                    reports.push(format!("mcp server {:?}: {reason}", server.name()));
                    continue;
                }
            };
            let secrets = env
                .iter()
                .map(|(_, value)| value.clone())
                .collect::<Vec<_>>();
            match McpServer::connect(server, env, secrets) {
                Ok(started) => {
                    reports.extend(
                        started
                            .drops()
                            .iter()
                            .map(|reason| format!("mcp server {:?}: {reason}", started.name())),
                    );
                    connected.push(Arc::new(started));
                }
                Err(reason) => reports.push(reason),
            }
        }
        Some(Arc::new(Self {
            servers: Arc::new(connected),
            reports: Arc::new(reports),
            policy,
            consent: operator.map_or(Consent::Absent, Consent::Operator),
            caller: caller.clone(),
            cancellation,
        }))
    }

    /// The same started servers under one turn's consent: the session
    /// composes its servers once and hands every turn its own mode, consent,
    /// and cancellation. No server is started and no request is made — this
    /// is the second half of the value above, not a second connection.
    pub(crate) fn with_consent(
        &self,
        policy: ApprovalPolicy,
        consent: Consent,
        caller: Caller,
        cancellation: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            servers: Arc::clone(&self.servers),
            reports: Arc::clone(&self.reports),
            policy,
            consent,
            caller,
            cancellation,
        })
    }

    /// Whether this client started no server at all. A client with none is still
    /// a client — the reports say why — but it advertises nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// The connection pass off the async runtime's worker: starting a process
    /// and reading its handshake is blocking work with a bounded deadline, and
    /// it must not hold the runtime while it runs.
    pub(crate) async fn connect_async(
        servers: ResolvedMcpServers,
        resolver: Arc<dyn SecretResolver>,
        policy: ApprovalPolicy,
        operator: Option<Arc<dyn Operator>>,
        caller: Caller,
        cancellation: CancellationToken,
    ) -> Option<Arc<Self>> {
        tokio::task::spawn_blocking(move || {
            Self::connect(
                &servers,
                resolver.as_ref(),
                policy,
                operator,
                &caller,
                cancellation,
            )
        })
        .await
        .ok()
        .flatten()
    }

    /// What the connection pass has to report: servers that could not be
    /// started, tools that were dropped, listings that were cut short.
    pub(crate) fn reports(&self) -> &[String] {
        &self.reports
    }

    /// The definitions this client advertises, in server-then-listing order.
    pub(crate) fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools().map(definition).collect()
    }

    /// Every tool this client offers, in advertisement order.
    fn tools(&self) -> impl Iterator<Item = &super::server::McpTool> {
        self.servers.iter().flat_map(|server| server.tools())
    }

    /// Whether `name` is one of this client's tool names. The one table the
    /// advertisement, the dispatch, and the approval routing all read.
    pub(crate) fn owns(name: &str) -> bool {
        name::split(name).is_some()
    }

    /// The refusal this client's mode earns for every call, or `None` where the
    /// mode serves one. Read by both `approve` and `call`, so the two cannot
    /// disagree about whether a call may run.
    fn refusal(&self) -> Option<&'static str> {
        match self.policy {
            ApprovalPolicy::Bypass => None,
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits
                if !matches!(self.consent, Consent::Absent) =>
            {
                None
            }
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => Some(MCP_APPROVAL_UNAVAILABLE),
            ApprovalPolicy::ReadOnly => Some(READ_ONLY_REFUSAL),
            ApprovalPolicy::Never => Some(NEVER_REFUSAL),
        }
    }

    /// The server that owns `name`, and its entry for the tool.
    fn entry(&self, name: &str) -> Option<(&Arc<McpServer>, &super::server::McpTool)> {
        let (server, tool) = name::split(name)?;
        let owner = self
            .servers
            .iter()
            .find(|candidate| candidate.name() == server)?;
        let entry = owner
            .tools()
            .iter()
            .find(|candidate| candidate.tool == tool)?;
        Some((owner, entry))
    }
}

/// Resolves one server's `env` references into the values the child is handed,
/// in name order. A reference that cannot be resolved fails the whole server:
/// starting it without a credential it asked for would run a program in a
/// state its configuration never described.
fn resolve_env(
    server: &saya_config::ResolvedMcpServer,
    resolver: &dyn SecretResolver,
) -> Result<Vec<(String, String)>, String> {
    let mut env = Vec::with_capacity(server.env().len());
    for (key, reference) in server.env() {
        let value = resolver
            .resolve(reference)
            .map_err(|error| format!("env {key} could not be resolved: {error}"))?;
        env.push((key.clone(), value.expose().to_owned()));
    }
    Ok(env)
}
