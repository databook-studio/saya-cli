//! The child composition boundary: the factory composes one child's provider,
//! toolset, definitions and approval from ONE immutable authority view. The
//! approval decider wraps the child's coarse effect gate with exact write-path
//! admission, so a refused write is denied before execution; the executor
//! keeps the same admission as defense in depth.

use std::sync::Arc;

use saya_config::{ResolvedAi, SecretResolver};
use saya_harness::coding::child::{
    ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError,
};
use saya_harness::runner::RunProgram;

use super::{
    ChildToolsError, TrustedCheckoutPair, child_toolset, child_toolset_with_runner, decider,
};
use crate::agent::provider;

/// Composes one verified, confined child's runtime dependencies from pieces
/// already prepared by the composition root: a trusted checkout pairing, a
/// runner already confined by `ConfinedCheckout::runner_tool(..)` (or none),
/// the resolved AI configuration, and the secret resolver providers need.
///
/// The approval decider is the child's coarse effect gate wrapped with exact
/// write-path admission (`decider::ExactWriteChildApproval`): a refused write
/// is denied before execution, so it admits no permit and settles no
/// uncertain effect; the executor keeps the same admission as defense in
/// depth.
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
    coding: Option<Arc<crate::commands::coding::checks::ChildCodingOperations>>,
}

impl CliChildFactory {
    pub(crate) fn with_coding(
        mut self,
        operations: Arc<crate::commands::coding::checks::ChildCodingOperations>,
    ) -> Self {
        self.coding = Some(operations);
        self
    }

    /// Assembles the factory from pieces prepared by `compose_child` in the
    /// same use-time verification flow.
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
            coding: None,
        }
    }
}

impl ChildFactory for CliChildFactory {
    /// Builds the provider, toolset, definitions and approval for one child
    /// from its immutable authority view.
    ///
    /// The approval decider wraps the child's coarse effect gate with exact
    /// write-path admission, so an unassigned write is denied before any
    /// permit is admitted; the executor's identical admission stays as
    /// defense in depth.
    ///
    /// Commands exist only where the view grants them: a command scope on the
    /// view without a proven runner is refused, and a prepared runner is
    /// ignored when the view grants no command scope.
    fn build(
        &self,
        spec: &ChildExecutionSpec,
        cancellation: &saya_agent::CancellationToken,
    ) -> Result<ChildComponents, ChildFactoryError> {
        let view = spec.authority();
        let wants_commands =
            view.capabilities().runner.is_some() || view.capabilities().interpreter.is_some();
        let (mut definitions, mut tools) = match (&self.runner, wants_commands) {
            (Some(runner), true) => child_toolset_with_runner(
                view,
                &self.pair,
                Arc::new(runner.for_invocation(cancellation.clone())),
            ),
            (None, true) => Err(ChildToolsError::ConfinementMissing),
            (_, false) => child_toolset(view, &self.pair),
        }
        .map_err(ChildFactoryError::new)?;
        if let Some(coding) = &self.coding {
            definitions.extend(super::super::coding::child::definitions(wants_commands));
            tools.attach_coding(Arc::clone(coding));
        }
        let mut ai = self.ai.clone();
        ai.model = spec.request().model.clone();
        let provider =
            provider::build(&ai, self.resolver.as_ref()).map_err(ChildFactoryError::new)?;
        Ok(ChildComponents::new(
            provider,
            Box::new(tools),
            definitions,
            Box::new(decider::ExactWriteChildApproval::new(
                view,
                self.pair.checkout_id().clone(),
            )),
        ))
    }
}
