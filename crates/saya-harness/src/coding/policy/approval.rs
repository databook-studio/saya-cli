//! The child's approval decider.
//!
//! Constructible only from an already-narrowed [`ChildAuthorityView`].
//! External side effects are always denied. Selected database identities admit
//! only the standard read-only database tools; other read-shaped effects
//! pass; `write`/`edit`/`delete` pass only when the view carries the write
//! capability (a write-capable view writes anywhere in the shared working
//! checkout, so there is no per-path check at this level). `rewind` is the
//! root's own checkpoint restore, a workspace write like `write` and `edit`,
//! and it follows the same capability; no helper advertises it. `shell` and
//! `bash` are admitted for every child here: a helper's shell follows the
//! root's Ask/Bypass rule and its host-versus-sandbox decision belongs to the
//! CLI shell policy, not to this gate.
//! Call arguments are NEVER read, so a parent prompt or a model-supplied
//! "grant" cannot widen anything. There is no session-grant, journal or
//! prompt input: this is the coarse effect-level gate, and per-call exact
//! admission (path / program) stays in the executor.

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, LocalStateEffect, ToolDefinition, read_only_permits};

use super::ChildAuthorityView;

/// Coarse effect-level gate derived from the child's authority view.
#[derive(Debug)]
pub struct ChildApprovalDecider {
    workspace_writes: bool,
    database_reads: bool,
}

impl ChildApprovalDecider {
    /// Narrows to exactly what the already-narrowed view grants.
    pub fn from_view(view: &ChildAuthorityView) -> Self {
        Self {
            database_reads: !view.database_profiles().is_empty(),
            workspace_writes: view.workspace_write(),
        }
    }
}

#[async_trait]
impl ApprovalDecider for ChildApprovalDecider {
    async fn approve(&self, tool: &ToolDefinition, _arguments: &serde_json::Value) -> bool {
        if tool.effect.external_side_effect {
            return false;
        }
        if tool.effect.database_data {
            // The standard read-only database tools: the SQL safety layer
            // inside the tool is what refuses a write statement, so this gate
            // admits the call only where the view selected a profile.
            return self.database_reads
                && tool.read_only
                && matches!(tool.name.as_str(), "schema_discovery" | "bounded_sql_query");
        }
        if read_only_permits(&tool.effect) {
            return true;
        }
        if matches!(tool.effect.local_state, LocalStateEffect::WriteWorkspace) {
            return match tool.name.as_str() {
                "write" | "edit" | "delete" | "rewind" => self.workspace_writes,
                // A helper's shell follows the root's Ask/Bypass rule: whether
                // the host or the sandbox path runs the call is the CLI shell
                // policy's decision, so this gate admits it for every child.
                "shell" | "bash" => true,
                _ => false,
            };
        }
        false
    }

    fn refusal_detail(
        &self,
        _tool: &ToolDefinition,
        _arguments: &serde_json::Value,
    ) -> Option<String> {
        Some("the child authority does not grant this tool".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::policy::{
        ChildAuthorityRequest, ChildRole, PlatformSupport, RootDelegationInput, RootDelegationScope,
    };
    use saya_agent::{ToolConcurrency, ToolEffect};
    use saya_types::{
        AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, RepositoryId, Sha256Digest,
        SnapshotId,
    };

    /// A helper view with the shared working checkout and the write
    /// capability: the shape that admits `write`, `edit` and `delete`.
    fn writable_helper_view() -> ChildAuthorityView {
        let checkout = ManagedCheckoutId::parse("checkout").unwrap();
        let mut capabilities = Capabilities::default();
        capabilities.workspace_write = true;
        RootDelegationScope::from_input(RootDelegationInput {
            job_id: JobId::parse("job").unwrap(),
            agent_id: AgentId::parse("agent").unwrap(),
            invocation_id: InvocationId::parse("invocation").unwrap(),
            repository_id: RepositoryId::parse("repository").unwrap(),
            snapshot_id: SnapshotId::parse("snapshot").unwrap(),
            managed_checkout_id: Some(checkout),
            policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
            revocation_epoch: 7,
            capabilities: capabilities.clone(),
            database_profiles: Vec::new(),
        })
        .unwrap()
        .narrow(
            ChildAuthorityRequest::new(ChildRole::Implementer, capabilities, Vec::new()).unwrap(),
            PlatformSupport {
                workspace_edits: true,
                network_fetch: false,
                database_reads: false,
            },
        )
        .unwrap()
    }

    /// A read-only (`explore`) helper on the same shared working checkout: it
    /// holds the checkout but no write capability.
    fn read_only_helper_view() -> ChildAuthorityView {
        let checkout = ManagedCheckoutId::parse("checkout").unwrap();
        RootDelegationScope::from_input(RootDelegationInput {
            job_id: JobId::parse("job").unwrap(),
            agent_id: AgentId::parse("agent").unwrap(),
            invocation_id: InvocationId::parse("invocation").unwrap(),
            repository_id: RepositoryId::parse("repository").unwrap(),
            snapshot_id: SnapshotId::parse("snapshot").unwrap(),
            managed_checkout_id: Some(checkout),
            policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
            revocation_epoch: 7,
            capabilities: Capabilities::default(),
            database_profiles: Vec::new(),
        })
        .unwrap()
        .narrow(
            ChildAuthorityRequest::new(ChildRole::Analyst, Capabilities::default(), Vec::new())
                .unwrap(),
            PlatformSupport {
                workspace_edits: true,
                network_fetch: false,
                database_reads: false,
            },
        )
        .unwrap()
    }

    fn write_shaped(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_owned(),
            description: "a probe tool".to_owned(),
            read_only: false,
            parameters: serde_json::json!({ "type": "object" }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        }
    }

    /// A helper's shell follows the root's Ask/Bypass rule, so this gate
    /// admits `shell` and `bash` for a write-capable and for a read-only helper
    /// alike; the write tools follow the write capability.
    #[tokio::test]
    async fn a_helper_gets_the_shell_and_writes_follow_the_capability() {
        let writable = writable_helper_view();
        assert!(writable.managed_checkout_id().is_some());
        assert!(writable.workspace_write());
        let decider = ChildApprovalDecider::from_view(&writable);
        let no_args = serde_json::json!({});

        assert!(decider.approve(&write_shaped("shell"), &no_args).await);
        assert!(decider.approve(&write_shaped("bash"), &no_args).await);
        assert!(decider.approve(&write_shaped("write"), &no_args).await);
        assert!(decider.approve(&write_shaped("edit"), &no_args).await);
        assert!(decider.approve(&write_shaped("delete"), &no_args).await);
        assert!(decider.approve(&write_shaped("rewind"), &no_args).await);

        let read_only = read_only_helper_view();
        assert!(!read_only.workspace_write());
        let decider = ChildApprovalDecider::from_view(&read_only);
        assert!(decider.approve(&write_shaped("shell"), &no_args).await);
        assert!(!decider.approve(&write_shaped("write"), &no_args).await);
        assert!(!decider.approve(&write_shaped("edit"), &no_args).await);
        assert!(!decider.approve(&write_shaped("delete"), &no_args).await);
        assert!(!decider.approve(&write_shaped("rewind"), &no_args).await);
    }

    /// `run_program` is not a tool here: it has no arm in this gate.
    #[tokio::test]
    async fn run_program_is_no_longer_admitted_by_any_view() {
        let decider = ChildApprovalDecider::from_view(&writable_helper_view());
        assert!(
            !decider
                .approve(&write_shaped("run_program"), &serde_json::json!({}))
                .await
        );
    }
}
