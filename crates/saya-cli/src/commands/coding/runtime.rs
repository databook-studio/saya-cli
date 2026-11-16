use super::{CodingApplication, CodingJobRequest};
use saya_agent::CancellationToken;
use saya_harness::coding::{
    budget::JobBudgetLedger, context::OriginalProjectContext, local::JobBaseline,
};
use saya_types::{RepositoryId, Sha256Digest};
use std::{path::PathBuf, sync::Arc};

/// The one live owner of an approved native job. No grants are deserialized.
pub struct NativeCodingJob {
    pub(super) application: Arc<CodingApplication>,
    pub(super) request: CodingJobRequest,
    pub(super) plan: JobBaseline,
    pub(super) repository_id: RepositoryId,
    pub(super) policy_digest: Sha256Digest,
    pub(super) epoch: u64,
    pub(super) ledger: JobBudgetLedger,
    pub(super) project: OriginalProjectContext,
    pub(super) records: PathBuf,
    /// The job's shared before-image store, for an in-place job. Root and
    /// helpers capture the first bytes each edit saw here; `None` for a
    /// managed checkout, whose rewind store already holds the baseline.
    pub(super) before_images:
        Option<Arc<saya_harness::coding::local::before_image::BeforeImageStore>>,
    /// The one-live-job lease over an in-place root. Held for the job's
    /// lifetime; `None` for a managed checkout. The leading underscore says
    /// the field is held for its drop, not read.
    pub(super) _in_place_lease: Option<super::in_place_lease::InPlaceLease>,
    /// The one `[tools] allow` list this job runs under: what its request
    /// stated, or — when it stated none — the default that narrows nothing.
    /// Resolved once here from the persisted request, so a resume runs the set
    /// the job started with and a helper can derive its own from it.
    pub(super) tools: saya_config::ResolvedTools,
    pub(super) root_steering: saya_agent::AgentSteering,
    pub(super) root_mailbox: tokio::sync::Mutex<Option<saya_agent::AgentMailbox>>,
    pub(super) observation_sequence: std::sync::atomic::AtomicU64,
    pub(super) observation_failed: std::sync::atomic::AtomicBool,
    /// How much tool-call transcript this job's journal has recorded, and
    /// whether it has already said that recording stopped.
    pub(super) transcripts: super::transcript::TranscriptBudget,
    /// Serializes one tool event's attach, append and marker check, so a
    /// concurrent call cannot record a body after the marker named the first
    /// dropped body. The append itself already serializes on the file lock;
    /// this keeps the budget's ordinals in that same order.
    pub(super) transcript_lock: std::sync::Mutex<()>,
    pub(super) failure: std::sync::Mutex<super::failure::FailureState>,
    /// Why the end-of-run owner preparation produced no ready candidate: the
    /// closed reason the advisory view names, set once by `prepare_at_finish`.
    pub(super) preparation: std::sync::Mutex<Option<super::finish_direct::PreparationRefusal>>,
    /// Children started by `delegate` in this run (a one-path job allows one).
    pub(super) delegated_children: std::sync::atomic::AtomicUsize,
    /// W's invalidation epoch: bumped on every root tool ATTEMPT that could
    /// have changed the root working checkout, whatever its outcome. This is
    /// the conservative key of the advisory readiness cache.
    pub(super) root_w_epoch: std::sync::atomic::AtomicU64,
    /// The advisory readiness cache (`readiness_cache`): W's delta and the
    /// current generation's evidence binding, reused until the key moves.
    pub(super) readiness_cache: super::readiness_cache::ReadinessCache,
    /// The working-checkout proof this start flow produced (a completed
    /// create or a resume verification), keyed by W's invalidation epoch. The
    /// advisory readiness spends it only while the epoch is unchanged; every
    /// root tool attempt moves the epoch and forces a live verification.
    pub(super) root_checkout_seed:
        std::sync::Mutex<Option<(u64, Arc<saya_harness::coding::worktree::VerifiedCheckout>)>>,
    pub(super) operation: tokio::sync::Mutex<()>,
    pub(super) active: tokio::sync::Mutex<Option<super::controls::LiveSession>>,
    pub(super) progress:
        tokio::sync::broadcast::Sender<saya_harness::coding::coordinator::ChildEvent>,
    pub(super) cancellation: CancellationToken,
    /// The operator's approver for every egressing call this job makes — a
    /// `shell` run outside the sandbox, and a `web_fetch` — serialised so one
    /// prompt is pending at a time whatever asks. A refusal is a normal
    /// denial. `None` means no operator is present, so such a shell run falls
    /// back to the sandbox (or is refused unasked) and a fetch is refused
    /// unasked.
    pub(super) operator: Option<Arc<dyn crate::agent::tools::consent::Operator>>,
    /// The one egress this job's web tools run through: the session's own
    /// fetch policy shape over the production wire, plus the search backend
    /// the user configured — which is absent unless they stated one. Built
    /// once here, shared by the root and every helper, or `None` where the
    /// mode reaches no host or the wire could not be built — in which case no
    /// surface advertises a web tool at all.
    pub(super) web_egress: Option<Arc<crate::agent::tools::web::WebEgress>>,
    /// The job's plan gate: present only for a job its request stated
    /// plan-first, and then closed until an operator approves a submitted
    /// plan. `None` is every job that never entered plan mode — its root
    /// advertises the composition's whole surface from the first request.
    pub(super) plan_gate: Option<Arc<crate::agent::tools::coding::plan_gate::PlanGate>>,
    /// This run's bounded lifecycle spans, handed over by `create`.
    pub(super) lifecycle: Arc<super::lifecycle::LifecycleTimeline>,
    /// The advisory readiness computed off the critical path, when this run
    /// can start one safely: no hooks and no MCP server can change W without
    /// moving the cache key. `None` means the outcome computes it as before.
    pub(super) readiness_precompute:
        std::sync::Mutex<Option<Arc<super::readiness_cache::ReadinessPrecompute>>>,
    /// Set by a headless `start`: nobody waits on a ready candidate, so the
    /// end-of-run checkout-and-integrate pass is skipped and `saya job review`
    /// prepares one on demand.
    pub(super) deferred_preparation: std::sync::atomic::AtomicBool,
}

impl NativeCodingJob {
    pub fn subscribe(
        &self,
    ) -> tokio::sync::broadcast::Receiver<saya_harness::coding::coordinator::ChildEvent> {
        self.progress.subscribe()
    }
    pub fn job_id(&self) -> &saya_types::JobId {
        self.request.job_id()
    }
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
    /// The job's private state directory: where the job's records live, and
    /// where a shell result too large for the model's inline bound saves its
    /// full captured output.
    pub(crate) fn records(&self) -> &std::path::Path {
        &self.records
    }
    /// The job's shared before-image store, for an in-place job.
    pub(crate) fn before_images(
        &self,
    ) -> Option<Arc<saya_harness::coding::local::before_image::BeforeImageStore>> {
        self.before_images.clone()
    }
    /// The tool set this job runs under: the root's, and — minus `helpers` —
    /// every helper's. Carried on the job, not read from the runtime, so a
    /// resumed job runs the set its own request persisted.
    pub(crate) fn tools(&self) -> &saya_config::ResolvedTools {
        &self.tools
    }
    /// The request text this job was started with. The optional tools are
    /// advertised from what it asks for — a URL, a helper — so the root reads
    /// it here rather than from the loop's first message.
    pub(crate) fn task(&self) -> &str {
        self.request.task()
    }
    /// The helper agent types this job's `helpers` tool advertises: the two
    /// built-ins, plus whatever trusted `[agents]` added and narrowed.
    pub(crate) fn helper_agents(&self) -> &saya_config::HelperAgents {
        &self.application.runtime.resolved.agents.types
    }
    /// The approval mode the root's `shell` reads when it decides whether a
    /// run outside the sandbox must be put to the operator. Carried from the
    /// application, so the job answers with the mode its surface was built in.
    pub(crate) fn approval_policy(&self) -> saya_agent::ApprovalPolicy {
        self.application.approval_policy
    }
    /// The composed runtime the job was built over: the resolved config and
    /// the secret values saya already holds, read where the host policy for a
    /// host shell run is built.
    pub(crate) fn runtime_config(&self) -> &crate::config::runtime::RuntimeConfig {
        &self.application.runtime
    }
    /// The operator's approver for an egressing call — a `shell` run outside
    /// the sandbox, or a `web_fetch` — when one is present. Without it such a
    /// run falls back to the sandbox under `ask` (and is refused when no
    /// sandbox is proven), and a fetch is refused with the reason naming the
    /// flag that would reach the network.
    pub(crate) fn operator(&self) -> Option<Arc<dyn crate::agent::tools::consent::Operator>> {
        self.operator.clone()
    }
    /// The `[mcp_servers]` table this job was built over. Read from the
    /// runtime rather than carried on the job: it is a protected setting, so
    /// no layer of the job's own request could have stated it, and every
    /// surface of one process must see the same servers.
    pub(crate) fn mcp_servers(&self) -> &saya_config::ResolvedMcpServers {
        &self.application.runtime.resolved.mcp_servers
    }
    /// The job's one web egress, shared by the root and every helper, or
    /// `None` where none could be composed.
    pub(crate) fn web_egress(&self) -> Option<Arc<crate::agent::tools::web::WebEgress>> {
        self.web_egress.clone()
    }
    /// The job's plan gate, or `None` for a job that never entered plan mode.
    /// Carried on the job, not read from the runtime, so a resumed plan-first
    /// job runs under the gate its own request persisted.
    pub(crate) fn plan_gate(
        &self,
    ) -> Option<Arc<crate::agent::tools::coding::plan_gate::PlanGate>> {
        self.plan_gate.clone()
    }
    /// Whether an operator surface is attached to this job: the interactive
    /// panel that answers a `shell` or a `web_fetch` prompt. It is the same
    /// surface that can answer a clarification question and a submitted plan,
    /// so a job without one advertises no ask — a headless run must never stop
    /// for a question nobody can answer — and is refused `--plan` at start.
    pub(crate) fn operator_present(&self) -> bool {
        self.operator.is_some()
    }
    /// This run's lifecycle timeline.
    pub(crate) fn lifecycle(&self) -> &super::lifecycle::LifecycleTimeline {
        &self.lifecycle
    }

    /// The same timeline as an owned handle, for a background task that
    /// records a timer after the call that started it has returned.
    pub(crate) fn lifecycle_arc(&self) -> Arc<super::lifecycle::LifecycleTimeline> {
        Arc::clone(&self.lifecycle)
    }
}
