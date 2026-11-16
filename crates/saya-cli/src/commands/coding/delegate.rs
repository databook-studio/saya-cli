use super::task_flow::MAX_HELPER_ASSIGNMENT_BYTES;
use super::{CodingError, NativeCodingJob, create::digest, task_history};
use saya_harness::coding::coordinator::ScheduledChild;
use saya_store::NewCodingChild;
use saya_types::{AgentId, InvocationId, ManagedCheckoutId, Sha256Digest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The `helpers` description cap: the tool schema's `maxLength` and
/// `one_helper`'s validation both use it. The composed assignment is the
/// description, a blank line, and the prompt.
pub(crate) const MAX_HELPER_DESCRIPTION_BYTES: usize = 128;

/// The blank line `one_helper` puts between the description and the prompt.
pub(crate) const HELPER_ASSIGNMENT_SEPARATOR_BYTES: usize = 2;

// Every assignment the `helpers` tool admits must compose. The tool admits a
// prompt up to `MAX_HELPER_ASSIGNMENT_BYTES`, a description up to
// `MAX_HELPER_DESCRIPTION_BYTES`, and the separator, so the child brief bound
// must cover their sum: raising a tool cap without the harness bound fails the
// build.
const _: () = assert!(
    MAX_HELPER_ASSIGNMENT_BYTES + MAX_HELPER_DESCRIPTION_BYTES + HELPER_ASSIGNMENT_SEPARATOR_BYTES
        <= saya_harness::coding::context::MAX_CHILD_TASK_BYTES
);

/// The default `subagent_type` a task recorded before the field existed loads
/// as: the write-capable one, which is what every helper was.
fn general_subagent_type() -> String {
    "general".to_owned()
}

/// One helper's assignment. `subagent_type` decides the helper's role and
/// capability: `explore` is read-only, `general` implements one piece.
/// `write_paths` and `dependencies` are kept as serde fields so
/// `records/tasks.jsonl` written before the shared tree still parses; nothing
/// produces them (helpers write W directly).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingTask {
    pub agent_id: AgentId,
    pub task: String,
    #[serde(default = "general_subagent_type")]
    pub subagent_type: String,
    #[serde(default)]
    pub write_paths: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<AgentId>,
}
impl NativeCodingJob {
    /// Validates one complete assignment batch before it becomes durable or
    /// creates a child or budget reservation. Both `delegate` and `helpers`
    /// admit through this single function: the 32 KiB assignment cap, the id
    /// rules, the dependency-count cap, the 32-task limit, the helper type the
    /// job's registry advertises, and the task history bound. The task history
    /// is authoritative for admission, so a legacy ambiguity fails closed.
    pub(crate) fn validate_delegate_tasks(&self, tasks: &[CodingTask]) -> Result<(), CodingError> {
        let (recorded, _) = task_history::read_tasks(&self.records)?;
        if recorded.len().saturating_add(tasks.len()) > task_history::MAX_TASKS {
            return Err(CodingError::InvalidRequest);
        }
        let mut agents = BTreeSet::new();
        let helper_types = self.helper_agents();
        for task in tasks {
            if task.task.len() > MAX_HELPER_ASSIGNMENT_BYTES {
                return Err(CodingError::AssignmentTooLong);
            }
            // The type must be one this job advertises. A name it does not hold
            // is refused by name, with the ones it does: the model asked for a
            // helper this job cannot compose, and nothing has been registered
            // or reserved yet.
            if helper_types.get(&task.subagent_type).is_none() {
                return Err(CodingError::UnknownSubagentType {
                    requested: task.subagent_type.clone(),
                    known: helper_types.known_names(),
                });
            }
            if task.agent_id.as_str() == "coordinator"
                || task.task.trim().is_empty()
                || task.dependencies.len() > 8
                || !agents.insert(task.agent_id.clone())
                || recorded.contains_key(&task.agent_id)
            {
                return Err(CodingError::InvalidRequest);
            }
        }
        Ok(())
    }

    /// Registers one task's child and composes it onto the root's working
    /// checkout W. Helpers share W, so no per-child checkout is created:
    /// the child's checkout id is W's. The registration is recorded in
    /// `registered` as soon as it exists, so the caller can take it back if
    /// this or a later task fails.
    pub(super) async fn prepare_child(
        &self,
        task: CodingTask,
        registered: &mut Vec<(AgentId, InvocationId)>,
    ) -> Result<StagedChild, CodingError> {
        if task.agent_id.as_str() == "coordinator"
            || task.task.trim().is_empty()
            || task.task.len() > MAX_HELPER_ASSIGNMENT_BYTES
            || task.dependencies.len() > 8
        {
            return Err(CodingError::InvalidRequest);
        }
        let checkout = self
            .ensure_root_checkout()
            .await?
            .ok_or(CodingError::ApprovalRequired)?;
        let identity =
            digest(format!("{}:{}", self.job_id().as_str(), task.agent_id.as_str()).as_bytes())?;
        let invocation = InvocationId::parse(&format!("inv-{}", &identity.as_str()[..32]))
            .map_err(CodingError::owner)?;
        let host = &self.application;
        host.store
            .create_coding_child(NewCodingChild {
                job_id: self.job_id().clone(),
                agent_id: task.agent_id.clone(),
                current_invocation_id: invocation.clone(),
            })
            .await
            .map_err(CodingError::owner)?;
        registered.push((task.agent_id.clone(), invocation.clone()));
        let agent = task.agent_id.clone();
        // The child is registered now: whatever fails from here on is past this
        // call's pre-staging window, so it must never read as a refusal that
        // proves no effect. `InvalidRequest` is reserved for the checks above,
        // which run before any registration, checkout, history line or ledger
        // event; a composition failure is an owner-side failure.
        let (node, scope) = match self
            .compose_owned_child(task, checkout.clone(), invocation.clone(), None, None, None)
            .await
        {
            Ok(composed) => composed,
            Err(CodingError::InvalidRequest) => {
                return Err(CodingError::owner(ChildCompositionRefused));
            }
            Err(error) => return Err(error),
        };
        Ok(StagedChild {
            agent,
            invocation,
            checkout,
            scope,
            node,
        })
    }
}

/// A registered child whose brief could not be composed. It is deliberately
/// not `InvalidRequest`: the registration already happened, so the failure is
/// past the pre-staging window and is not proof of no effect.
#[derive(Debug, thiserror::Error)]
#[error("the registered child's brief could not be composed")]
struct ChildCompositionRefused;

/// A child that is registered and composed, whose scope is not yet bound and
/// which no schedule has been given.
pub(super) struct StagedChild {
    pub(super) agent: AgentId,
    pub(super) invocation: InvocationId,
    pub(super) checkout: ManagedCheckoutId,
    pub(super) scope: Sha256Digest,
    pub(super) node: ScheduledChild,
}
