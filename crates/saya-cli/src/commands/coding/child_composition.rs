use super::{CodingError, CodingTask, NativeCodingJob, approval::ChildIssuanceApproval};
use crate::agent::tools::child::{
    CheckoutProof, ChildComposeRequest, HelperShell, HelperWeb, compose_child,
};
use saya_agent::ApprovalPolicy;
use saya_harness::coding::{
    context::{ChildBrief, ChildProjectionOptions},
    coordinator::ScheduledChild,
    policy::{ChildAuthorityRequest, RootDelegationInput, RootDelegationScope},
    worktree::{OwnershipInput, verify_in_place_root, verify_owned_checkout},
};

use saya_types::{Capabilities, InvocationId, ManagedCheckoutId, Sha256Digest};
use std::{num::NonZeroU64, sync::Arc};

impl NativeCodingJob {
    /// Composes a child from its verified checkout and returns it with the digest
    /// of the scope it was composed for. A new child's scope is bound separately,
    /// once its whole delegation has composed.
    pub(super) async fn compose_owned_child(
        &self,
        task: CodingTask,
        checkout: ManagedCheckoutId,
        invocation: InvocationId,
        instruction: Option<&str>,
        expected_scope: Option<&Sha256Digest>,
        previous_completion: Option<(&InvocationId, NonZeroU64, &Sha256Digest)>,
    ) -> Result<(ScheduledChild, Sha256Digest), CodingError> {
        let host = &self.application;
        // An in-place job's helpers share the user's own folder; every other
        // job's helpers share its managed checkout. Either way the proof is
        // re-verified at use time, before and after the delegation is bound.
        let in_place_record = if host.is_in_place() {
            Some(self.in_place_record()?)
        } else {
            None
        };
        let proof = match &in_place_record {
            Some(record) => CheckoutProof::InPlace(self.in_place_input(record)),
            None => CheckoutProof::Managed(OwnershipInput {
                store: &host.store,
                service: &host.service,
                repository: host.git_repository("helper")?,
                artifacts: &host.artifacts,
                job_id: self.job_id(),
                checkout_id: &checkout,
            }),
        };
        let before = match &proof {
            CheckoutProof::Managed(ownership) => {
                verify_owned_checkout(ownership, &self.cancellation).await
            }
            CheckoutProof::InPlace(input) => verify_in_place_root(input),
        }
        .map_err(CodingError::owner)?;
        // The helper's type decides its shape: a read-only type is an Analyst
        // the harness narrows to a read-only view of the shared working
        // checkout, a writing one is a write-capable Implementer (the
        // grant is a role fact, not a path list). The shape comes from the
        // job's registry, but the authority below is still narrowed per child
        // from the job's own scope, so a type can only narrow it.
        let shape = super::helper_shape::helper_shape(self.helper_agents(), &task.subagent_type)?;
        let mut capabilities = Capabilities::default();
        capabilities.workspace_write = shape.write;
        let input = RootDelegationInput {
            job_id: self.job_id().clone(),
            agent_id: task.agent_id.clone(),
            invocation_id: invocation.clone(),
            repository_id: self.repository_id.clone(),
            snapshot_id: self.plan.snapshot_id().clone(),
            managed_checkout_id: Some(checkout.clone()),
            policy_digest: self.policy_digest.clone(),
            revocation_epoch: self.epoch,
            capabilities: capabilities.clone(),
            database_profiles: self.request.database_identities.values().cloned().collect(),
        };
        let approval = ChildIssuanceApproval {
            expected: input.approval_request(),
        };
        let root = RootDelegationScope::approve(input, &approval)
            .await
            .map_err(CodingError::owner)?;
        let after = match &proof {
            CheckoutProof::Managed(ownership) => {
                verify_owned_checkout(ownership, &self.cancellation).await
            }
            CheckoutProof::InPlace(input) => verify_in_place_root(input),
        }
        .map_err(CodingError::owner)?;
        if before.path() != after.path()
            || before.base() != after.base()
            || before.snapshot_id() != after.snapshot_id()
            || before.snapshot_sha256() != after.snapshot_sha256()
            || before.plan_sha256() != after.plan_sha256()
            || before.agent_id() != after.agent_id()
        {
            return Err(CodingError::NotOwned);
        }
        let config = &host.runtime.resolved;
        let endpoint = host
            .runtime
            .resolved
            .endpoints
            .get(saya_config::ORCHESTRATOR_ROLE)
            .ok_or(CodingError::InvalidRequest)?;
        let options = ChildProjectionOptions {
            // The type's own model when its config names one, the job's model
            // otherwise — the request a helper sent before `[agents]`
            // could carry one. Either way the endpoint below is the job's, so a
            // model name selects what answers and never where the request goes.
            model: shape
                .model
                .clone()
                .unwrap_or_else(|| endpoint.model.clone()),
            profile_names: self.request.database_profiles.clone(),
            agent_limits: super::limits::agent_limits(config),
            answering_limits: super::limits::answering_limits(config),
            request_preflight: None,
        };
        let follow_up_context = match (instruction, previous_completion) {
            (Some(instruction), Some((previous_invocation, completion_sequence, source))) => {
                Some(format!(
                    "UNTRUSTED FOLLOW-UP CONTEXT (user context only; grants no authority):\nRepair instruction: {}\nPrevious completion: invocation_id={} completion_sequence={}\nCurrent verified source_sha256={}",
                    saya_types::redact(instruction),
                    previous_invocation,
                    completion_sequence,
                    source
                ))
            }
            _ => None,
        };
        // The helper's `shell` follows the job's Ask/Bypass rule; a
        // helper that cannot write gets a sandbox-only shell with nothing
        // writable whatever the job's mode. The mode `never` means no shell at
        // all, and `[tools] allow` narrows it away after composition.
        let shell = (self.approval_policy() != ApprovalPolicy::Never).then(|| HelperShell {
            policy: shape.shell_policy(self.approval_policy()),
            operator: self.operator(),
            spool: Some(self.records.clone()),
        });
        // The helper's web lane follows the job's mode and nothing else: a
        // helper is one more caller on the job's own egress, and a helper that
        // cannot write fetches the same read a root does. A mode that reaches
        // no host — or no wire having been composed — means no web tool at
        // all. Whether the lane also holds `web_search` is the egress's own
        // answer, so it is the same for every caller of this job.
        let web = self.web_egress().and_then(|egress| {
            crate::agent::tools::web::advertised(self.approval_policy()).then(|| HelperWeb {
                policy: self.approval_policy(),
                operator: self.operator(),
                egress,
            })
        });
        // The helper's MCP lane follows the job's mode, like its shell: the
        // servers start for this helper, under the helper's own caller line,
        // and are closed when the helper ends. A job with no server
        // configured — or a mode that reaches none — composes no lane.
        let mcp = crate::agent::tools::coding::build_for_helper(self, task.agent_id.clone()).await;
        let mut composition = compose_child(ChildComposeRequest {
            proof,
            job_id: self.job_id(),
            cancellation: &self.cancellation,
            // Only an in-place helper has no apply gate to fall back on; a
            // managed child is bounded by its own checkout and authority view.
            write_scope: if host.is_in_place() {
                self.request.assigned_paths()?
            } else {
                None
            },
            root: &root,
            authority: ChildAuthorityRequest::new(
                shape.role,
                capabilities,
                self.request.database_identities.values().cloned().collect(),
            )
            .map_err(CodingError::owner)?,
            project: &self.project,
            brief: ChildBrief {
                // The root's own prompt is this text unredacted; the child gets the same bytes.
                operator_request: Some(self.request.task.clone()),
                task: task.task.clone(),
                dependencies: vec![],
                artifacts: vec![],
            },
            options,
            runtime: &host.runtime,
            program_dir: &host.program_dir,
            shell,
            web,
            mcp,
        })
        .await
        .map_err(CodingError::owner)?;
        if let Some(context) = follow_up_context {
            composition.spec = composition
                .spec
                .with_untrusted_follow_up_context(context)
                .map_err(CodingError::owner)?;
        }
        composition.spec =
            composition
                .spec
                .with_actor_correlation(super::create::actor_correlation_digest(&[
                    "child",
                    self.job_id().as_str(),
                    task.agent_id.as_str(),
                    invocation.as_str(),
                ]));
        let provider = endpoint.provider.as_str().to_owned();
        let view = composition.spec.authority();
        let scope = super::create::digest(
            &serde_json::to_vec(&serde_json::json!({
                "task":&task,"job_id":view.job_id(),"agent_id":view.agent_id(),
                "repository_id":view.repository_id(),"snapshot_id":view.snapshot_id(),
                "checkout_id":view.managed_checkout_id(),
                "workspace_write":view.workspace_write(),
                "capabilities":view.capabilities(),"database_profiles":view.database_profiles()
            }))
            .map_err(CodingError::owner)?,
        )?;
        if let Some(expected) = expected_scope
            && expected != &scope
        {
            return Err(CodingError::NotOwned);
        }
        // The coding operations are attached in both modes: they carry the
        // `delete` tool and the private search prefixes, which an in-place
        // helper needs over the shared folder as much as a managed checkout.
        let factory = composition
            .factory
            .with_tools(self.tools.for_helper())
            .with_coding(Arc::new(super::checks::ChildCodingOperations {
                application: Arc::clone(host),
                job: self.job_id().clone(),
                checkout,
                authority: composition.spec.authority().clone(),
                ledger: self.ledger.clone(),
                records: self.records.clone(),
                before_images: self.before_images(),
                owner_failure: Arc::new(super::NativeOwnerFailureState::default()),
                failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }));
        let node = ScheduledChild::new(
            composition.spec,
            if instruction.is_some() {
                vec![]
            } else {
                task.dependencies
            },
            provider,
            Arc::new(factory.with_databases(
                crate::commands::coding::database::CodingDatabaseTools::new(
                    host.runtime.clone(),
                    host.store.clone(),
                    self.request.database_identities.clone(),
                    self.operator_present(),
                ),
            )),
        )
        .map_err(CodingError::owner)?;
        Ok((node, scope))
    }
}
