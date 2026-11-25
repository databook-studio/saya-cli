//! The factory-installed child approval: the harness coarse effect gate plus
//! the per-call write admission, decided before execution so a refused write
//! carries no permit and settles no uncertain effect.
//!
//! The write admission does not narrow by path: a write-capable view writes
//! anywhere in the shared working checkout, so the exact check is the
//! checkout identity plus the capability. Reading arguments still only ever
//! narrows — a call whose `path` is not a workspace path is refused — and the
//! executor keeps the same admission as defense in depth, so the two can only
//! agree.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, ImmutableAuthorityRefusal, ToolDefinition, WORKSPACE_WRITE_TOKEN,
};
use saya_harness::coding::integration::AssignedPaths;
use saya_harness::coding::policy::{ChildApprovalDecider, ChildAuthorityView};
use saya_types::{ManagedCheckoutId, WorkspacePath};
use serde_json::Value;

use crate::agent::tools::WriteAsk;

/// Wraps the child's coarse decider with the one exact check the executor
/// would otherwise refuse mid-run: whether this call writes the working
/// checkout this view was narrowed for, and — for an in-place job — whether
/// the path is inside the approved `--write` scope.
pub(crate) struct ExactWriteChildApproval {
    coarse: ChildApprovalDecider,
    view: ChildAuthorityView,
    checkout_id: ManagedCheckoutId,
    write_scope: Option<AssignedPaths>,
    /// The job's write ask, when an operator is attached to it. `None` — a
    /// detached composition — leaves every admitted write as the capability
    /// decided it.
    ask: Option<Arc<WriteAsk>>,
    /// The session's grant store, when the composition states one: a held
    /// `workspace-write` grant pre-answers this family's call, so the exact
    /// admission stands aside and the workspace tool's containment stays the
    /// last word.
    grants: Option<saya_agent::SessionGrants>,
}

/// Whether the session's own grant store holds the workspace-write family: the
/// grant pre-answers the call the mode would have asked about, so the exact
/// write admission stands aside and containment decides the path.
pub(crate) fn granted_workspace_write(grants: Option<&saya_agent::SessionGrants>) -> bool {
    grants.is_some_and(|grants| grants.is_granted(WORKSPACE_WRITE_TOKEN))
}

impl ExactWriteChildApproval {
    pub(crate) fn new(view: &ChildAuthorityView, checkout_id: ManagedCheckoutId) -> Self {
        Self {
            coarse: ChildApprovalDecider::from_view(view),
            view: view.clone(),
            checkout_id,
            write_scope: None,
            ask: None,
            grants: None,
        }
    }

    /// States the session's grant store: a family it has already been granted
    /// is admitted here, and the workspace tool's own containment answers a
    /// path this decider would otherwise refuse.
    pub(crate) fn with_grants(mut self, grants: saya_agent::SessionGrants) -> Self {
        self.grants = Some(grants);
        self
    }

    /// Whether the session's own store pre-answers this family's calls.
    fn granted(&self) -> bool {
        granted_workspace_write(self.grants.as_ref())
    }

    /// Puts the job's write ask in front of a helper's admitted writes: a write
    /// the capability admits is then resolved by the job's mode, exactly as
    /// the root's is.
    pub(crate) fn with_ask(mut self, ask: Option<Arc<WriteAsk>>) -> Self {
        self.ask = ask;
        self
    }

    /// Bounds every admitted write to the in-place job's approved `--write`
    /// scope. `None` leaves a managed checkout unbounded by path.
    pub(crate) fn with_write_scope(mut self, scope: Option<AssignedPaths>) -> Self {
        self.write_scope = scope;
        self
    }

    /// Exact admission of one write call; `Err` covers a missing or
    /// malformed path argument, a write this view does not hold, and a path
    /// outside the in-place scope, because the executor would refuse every
    /// one of these before mutating.
    fn write_admission(&self, arguments: &Value) -> Result<(), ()> {
        // A held grant is the session's own consent for this family, so the
        // path's shape and the authority's answer are not this decider's to
        // make: the workspace tool refuses a path that escapes it, with its
        // own typed containment error. The `--write` scope below is the user's
        // ceiling and is never pre-answered.
        if !self.granted() {
            let path = arguments.get("path").and_then(Value::as_str).ok_or(())?;
            let path = WorkspacePath::try_from(path.to_owned()).map_err(|_| ())?;
            self.view
                .admit_workspace_write(&self.checkout_id)
                .map_err(|_| ())?;
            if let Some(scope) = &self.write_scope
                && !scope.contains(path.as_str())
            {
                return Err(());
            }
            return Ok(());
        }
        if let Some(scope) = &self.write_scope
            && let Some(path) = arguments.get("path").and_then(Value::as_str)
            && WorkspacePath::try_from(path.to_owned())
                .is_ok_and(|path| !scope.contains(path.as_str()))
        {
            return Err(());
        }
        Ok(())
    }

    /// The scope that refuses this call's path, if any. `None` means the path
    /// is not a well-formed workspace path (the authority refuses that) or it
    /// is inside the scope.
    fn denied_scope(&self, arguments: &Value) -> Option<&AssignedPaths> {
        let scope = self.write_scope.as_ref()?;
        let path = arguments.get("path").and_then(Value::as_str)?;
        WorkspacePath::try_from(path.to_owned())
            .ok()
            .filter(|path| !scope.contains(path.as_str()))
            .map(|_| scope)
    }

    fn authority_path_denied(&self, arguments: &Value) -> bool {
        let Some(path) = arguments.get("path").and_then(Value::as_str) else {
            return false;
        };
        if WorkspacePath::try_from(path.to_owned()).is_err() {
            return false;
        }
        (!self.granted() && self.view.admit_workspace_write(&self.checkout_id).is_err())
            || self.denied_scope(arguments).is_some()
    }
}

#[async_trait]
impl ApprovalDecider for ExactWriteChildApproval {
    /// The capability first — a write this view does not hold, or a path outside
    /// the approved scope, is refused before anything is asked — and then the
    /// job's write ask, when one is attached.
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        if !self.coarse.approve(tool, arguments).await {
            return false;
        }
        if !crate::agent::tools::workspace::is_write_tool(&tool.name) {
            return true;
        }
        if self.write_admission(arguments).is_err() {
            return false;
        }
        match &self.ask {
            Some(ask) => ask.resolve(arguments).await,
            None => true,
        }
    }

    /// A write this decider's ask would put to the operator reports so, so a
    /// caller can decide what an ask means before making it. A call the
    /// capability already refuses is never put to anyone, and a helper of a job
    /// with no operator attached reports no ask of its own.
    fn would_ask(&self, tool: &ToolDefinition, arguments: &Value) -> Option<bool> {
        if !crate::agent::tools::workspace::is_write_tool(&tool.name) {
            return None;
        }
        let ask = self.ask.as_ref()?;
        Some(ask.asks_for(arguments) && (self.granted() || self.write_admission(arguments).is_ok()))
    }

    fn refusal_detail(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        if crate::agent::tools::workspace::is_write_tool(&tool.name)
            && self.write_admission(arguments).is_err()
        {
            if let Some(scope) = self.denied_scope(arguments) {
                return Some(super::scope::outside_write_scope(scope));
            }
            return Some("the child authority does not admit this write".to_string());
        }
        self.coarse.refusal_detail(tool, arguments)
    }

    fn immutable_authority_refusal(
        &self,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> Option<ImmutableAuthorityRefusal> {
        (crate::agent::tools::workspace::is_write_tool(&tool.name)
            && self.authority_path_denied(arguments))
        .then_some(ImmutableAuthorityRefusal::OutsideEffectiveWritePaths)
    }
}
