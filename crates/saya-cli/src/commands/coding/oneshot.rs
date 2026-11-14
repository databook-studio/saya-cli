//! The one-shot operation shared by `saya -p` (and, in a later chunk, the
//! start surfaces): one consent decision, one in-place job, one result object.

use super::{
    CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, NativeCodingJob,
    progress_report, usage,
};
use saya_agent::{AgentOutput, ApprovalPolicy, CancellationToken};
use saya_types::JobId;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[path = "oneshot_result.rs"]
mod oneshot_result;
pub use oneshot_result::OneShotResult;

/// One headless request: the task, an optional id, the effective write scope,
/// and the consent it resolved to. The CLI builds `write_paths` from the
/// launch plan, so an edit-capable `-p` with no `--write` carries the whole
/// workspace (`.`) rather than an empty list.
#[derive(Debug, Clone)]
pub struct OneShotRequest {
    pub task: String,
    pub job_id: Option<String>,
    pub write_paths: Vec<String>,
    pub mode: OneShotMode,
}

/// The four consents `-p` accepts. `plan` is refused: it needs an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneShotMode {
    ReadOnly,
    Never,
    AcceptEdits,
    Bypass,
}

impl OneShotMode {
    /// The canonical name the stream states for this consent.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Never => "never",
            Self::AcceptEdits => "accept-edits",
            Self::Bypass => "bypass",
        }
    }
}

/// What a mode resolves to: the approval policy the job runs under and the
/// write scope it may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub policy: ApprovalPolicy,
    pub write_paths: Vec<String>,
}

/// Resolves the raw `--permission-mode` / `--approval-mode` values into one
/// mode. `-p` is ungated for `bypass` in both spellings, so `allow_bypass` is
/// accepted for the caller's seam but never consulted.
pub fn resolve_mode(
    permission_mode: Option<&str>,
    approval_mode: Option<&str>,
    allow_bypass: bool,
) -> Result<OneShotMode, String> {
    let _ = allow_bypass;
    if permission_mode.is_some() && approval_mode.is_some() {
        return Err(
            "state `--permission-mode` or `--approval-mode`, not both: they are two \
                    spellings of the same consent"
                .to_owned(),
        );
    }
    match permission_mode.or(approval_mode) {
        Some(stated) => mode_from(stated),
        None => Ok(OneShotMode::ReadOnly),
    }
}

fn mode_from(stated: &str) -> Result<OneShotMode, String> {
    match stated {
        // No operator is present, so `ask` would route the shell to a sandbox
        // that can write the folder: the unstated consent is the read-only one.
        "ask" | "default" | "read-only" => Ok(OneShotMode::ReadOnly),
        "never" => Ok(OneShotMode::Never),
        "accept-edits" | "acceptEdits" => Ok(OneShotMode::AcceptEdits),
        "bypass" | "bypassPermissions" => Ok(OneShotMode::Bypass),
        "plan" => Err(
            "plan mode needs an operator to answer the submitted plan, and `-p` has \
                       no interactive surface: drop `--permission-mode plan`"
                .to_owned(),
        ),
        other => Err(format!(
            "invalid permission mode `{other}`: expected ask, read-only, never, accept-edits, \
             or bypass"
        )),
    }
}

/// The policy and write scope a mode launches with. `has_rules` is P6's seam:
/// the resolved command-rule set is non-empty. The rules are read at shell
/// time by the lane, so they change no field of this plan.
pub fn launch_plan(
    mode: OneShotMode,
    stated_write: &[String],
    has_rules: bool,
) -> Result<LaunchPlan, String> {
    let _ = has_rules;
    match mode {
        OneShotMode::ReadOnly | OneShotMode::Never => {
            if !stated_write.is_empty() {
                return Err(
                    "`--write` needs an edit-capable mode: state `--permission-mode \
                            accept-edits` (or `--permission-mode bypass`)"
                        .to_owned(),
                );
            }
            let policy = if mode == OneShotMode::Never {
                ApprovalPolicy::Never
            } else {
                ApprovalPolicy::ReadOnly
            };
            Ok(LaunchPlan {
                policy,
                write_paths: Vec::new(),
            })
        }
        OneShotMode::AcceptEdits => Ok(LaunchPlan {
            policy: ApprovalPolicy::AcceptEdits,
            write_paths: default_scope(stated_write),
        }),
        OneShotMode::Bypass => Ok(LaunchPlan {
            policy: ApprovalPolicy::Bypass,
            write_paths: default_scope(stated_write),
        }),
    }
}

/// An edit-capable mode with no stated scope edits the folder it was started
/// in; a stated scope replaces that default.
fn default_scope(stated: &[String]) -> Vec<String> {
    if stated.is_empty() {
        vec![".".to_owned()]
    } else {
        stated.to_vec()
    }
}

/// Whether the stated policy is itself the launch's consent. `accept-edits`
/// and `bypass` answer the approve question; `ask` still asks.
pub fn consent_implied(policy: ApprovalPolicy) -> bool {
    matches!(policy, ApprovalPolicy::AcceptEdits | ApprovalPolicy::Bypass)
}

/// The write scope a launch under `policy` uses. An edit-capable consent
/// defaults an empty stated scope to the folder; every other policy keeps the
/// stated scope because its approve gate is unchanged.
pub fn launch_write_scope(policy: ApprovalPolicy, stated: &[String]) -> Vec<String> {
    let mode = match policy {
        ApprovalPolicy::AcceptEdits => OneShotMode::AcceptEdits,
        ApprovalPolicy::Bypass => OneShotMode::Bypass,
        _ => return stated.to_vec(),
    };
    launch_plan(mode, stated, false)
        .expect("an edit-capable mode always plans a scope")
        .write_paths
}

/// A fresh job id: `p<unix-millis>-<pid>`.
pub fn generate_job_id() -> JobId {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    JobId::parse(&format!("p{millis}-{}", std::process::id()))
        .expect("the generated job id is well formed")
}

/// Creates the job the plan approves. The application must already carry the
/// plan's approval policy.
pub async fn start(
    app: &Arc<CodingApplication>,
    request: OneShotRequest,
    plan: &LaunchPlan,
    token: CancellationToken,
) -> Result<Arc<NativeCodingJob>, CodingError> {
    let job_id = match request.job_id.as_deref() {
        Some(stated) => JobId::parse(stated).map_err(CodingError::owner)?,
        None => generate_job_id(),
    };
    // The request carries the effective scope the CLI built from the plan; a
    // library caller that left it empty keeps the plan's resolved default.
    let write_paths = if request.write_paths.is_empty() {
        plan.write_paths.clone()
    } else {
        request.write_paths.clone()
    };
    let job_request = CodingJobRequest::new(job_id, request.task, write_paths)?;
    let (proposed, baseline) = app.propose_with_plan_until(&job_request, &token).await?;
    let consent = CodingAuthorization::scoped(&proposed)?;
    app.create_for_start(job_request, baseline, &consent, token)
        .await
}

/// Builds the result object from the finished job and the run's outcome.
pub async fn finish(
    app: &CodingApplication,
    job: &NativeCodingJob,
    run: Result<AgentOutput, CodingError>,
    wall: Duration,
) -> OneShotResult {
    let failure = job.failure();
    let state = oneshot_result::state(app, job).await;
    let exit_code = oneshot_result::exit_code(state);
    // A logical-requests stop reaches the latch as a budget stop when the
    // ledger cancelled the run; the ledger's own dimension is what names the
    // turn backstop, so the subtype reads it when the primary class is generic.
    let backstop = job.backstop_failure().await;
    let naming = match failure.as_ref() {
        Some(failure) if failure.class == saya_types::CodingFailureClass::BudgetStopped => {
            backstop.as_ref().or(Some(failure))
        }
        failure => failure,
    };
    let subtype = oneshot_result::subtype(naming);
    // Usage comes from the durable transport spans, never from stream events.
    let usage = usage::read(app, job.job_id()).await.unwrap_or(Value::Null);
    let progress = progress_report::read(&job.records);
    let outcome = job.outcome().unwrap_or(Value::Null);
    let answer = match &run {
        Ok(output) => Some(output.answer.clone()),
        Err(_) => None,
    }
    .or_else(|| {
        outcome
            .get("answer")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    let failure_value = failure.as_ref().map_or(Value::Null, |failure| {
        serde_json::to_value(failure).unwrap_or(Value::Null)
    });
    let failure_class = failure
        .as_ref()
        .map_or(Value::Null, |failure| json!(failure.class.as_str()));
    let duration_api_ms = usage
        .get("model_wall_time_ms")
        .cloned()
        .unwrap_or(Value::Null);
    let total_cost_usd = usage.get("cost_usd").cloned().unwrap_or(Value::Null);
    let value = json!({
        "event": "job_result",
        "type": "result",
        "subtype": subtype,
        "is_error": subtype != "success",
        "session_id": job.job_id(),
        "job_id": job.job_id(),
        "result": answer.map_or(Value::Null, Value::String),
        "num_turns": progress.get("turns").cloned().unwrap_or(json!(0)),
        "duration_ms": u64::try_from(wall.as_millis()).unwrap_or(u64::MAX),
        "duration_api_ms": duration_api_ms,
        "total_cost_usd": total_cost_usd,
        "usage": usage,
        "outcome": outcome,
        "failure": failure_value,
        "failure_class": failure_class,
        "exit_code": exit_code,
    });
    OneShotResult { value, exit_code }
}

#[cfg(all(test, unix))]
#[path = "oneshot_tests.rs"]
mod oneshot_tests;
