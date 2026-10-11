//! The child composition boundary: the factory composes one child's provider,
//! toolset, definitions and approval from ONE immutable authority view. The
//! approval decider is the child's `ChildApprovalDecider`: it permits exactly
//! the effects the view grants (workspace writes, process runs); per-call
//! exact admission stays in the executor.

use std::sync::Arc;

use saya_config::{ResolvedAi, SecretResolver};
use saya_harness::coding::child::{
    ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError,
};
use saya_harness::coding::policy::ChildApprovalDecider;
use saya_harness::runner::RunProgram;

use super::{ChildToolsError, TrustedCheckoutPair, child_toolset, child_toolset_with_runner};
use crate::agent::provider;

/// Composes one verified, confined child's runtime dependencies from pieces
/// already prepared by the composition root: a trusted checkout pairing, a
/// runner already confined by `ConfinedCheckout::runner_tool(..)` (or none),
/// the resolved AI configuration, and the secret resolver providers need.
///
/// The approval decider is the child's `ChildApprovalDecider`: it permits
/// exactly the effects the view grants (workspace writes, process runs);
/// per-call exact admission stays in the executor.
pub(crate) struct CliChildFactory {
    /// The trusted pairing of the exact checkout id with the exact workspace
    /// root; the factory never re-derives it from model input.
    pair: TrustedCheckoutPair,
    /// A runner already proven to be confined, or `None` when no proven
    /// confinement was prepared. A `Some` here does not by itself grant
    /// commands: the child's authority view must also carry the scope.
    runner: Option<Arc<RunProgram>>,
    /// The resolved AI configuration; the child request's model replaces its
    /// model before the provider is built.
    ai: ResolvedAi,
    /// The resolver `provider::build` uses to turn secret references into
    /// provider credentials.
    resolver: Arc<dyn SecretResolver>,
}

impl CliChildFactory {
    /// Assembles the factory from already-prepared pieces. The composition
    /// flow itself (confine -> narrow -> project -> factory) is a later
    /// slice; this constructor only receives its results.
    pub(crate) fn new(
        pair: TrustedCheckoutPair,
        runner: Option<Arc<RunProgram>>,
        ai: ResolvedAi,
        resolver: Arc<dyn SecretResolver>,
    ) -> Self {
        Self {
            pair,
            runner,
            ai,
            resolver,
        }
    }
}

impl ChildFactory for CliChildFactory {
    /// Builds the provider, toolset, definitions and approval for one child
    /// from its immutable authority view.
    ///
    /// The approval decider is the child's `ChildApprovalDecider`: it permits
    /// exactly the effects the view grants (workspace writes, process runs);
    /// per-call exact admission stays in the executor.
    ///
    /// Commands exist only where the view grants them: a command scope on the
    /// view without a proven runner is refused, and a prepared runner is
    /// ignored when the view grants no command scope.
    fn build(&self, spec: &ChildExecutionSpec) -> Result<ChildComponents, ChildFactoryError> {
        let view = spec.authority();
        let wants_commands =
            view.capabilities().runner.is_some() || view.capabilities().interpreter.is_some();
        let (definitions, tools) = match (&self.runner, wants_commands) {
            (Some(runner), true) => child_toolset_with_runner(view, &self.pair, Arc::clone(runner)),
            (None, true) => Err(ChildToolsError::ConfinementMissing),
            (_, false) => child_toolset(view, &self.pair),
        }
        .map_err(ChildFactoryError::new)?;
        let mut ai = self.ai.clone();
        ai.model = spec.request().model.clone();
        let provider =
            provider::build(&ai, self.resolver.as_ref()).map_err(ChildFactoryError::new)?;
        Ok(ChildComponents::new(
            provider,
            Box::new(tools),
            definitions,
            Box::new(ChildApprovalDecider::from_view(view)),
        ))
    }
}
