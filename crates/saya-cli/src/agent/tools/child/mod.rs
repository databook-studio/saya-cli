//! The child tool surface: workspace tools bound to one verified managed checkout.

#[cfg(test)]
mod approval_test_support;
mod compose;
pub(crate) mod decider;
mod executor;
mod factory;
mod no_effect;
mod scope;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_allow;
#[cfg(test)]
mod tests_ask;
#[cfg(test)]
mod tests_database;
#[cfg(test)]
mod tests_edit;
#[cfg(test)]
mod tests_factory;
#[cfg(test)]
mod tests_lean_advertisement;
#[cfg(test)]
mod tests_loop;
#[cfg(test)]
mod tests_mcp;
#[cfg(test)]
mod tests_no_effect;
#[cfg(test)]
mod tests_scope;
#[cfg(test)]
mod tests_shell;
#[cfg(test)]
mod tests_stop;

pub(crate) use compose::{ChildComposeRequest, HelperShell, HelperWeb, compose_child};
// `ChildTools` is exposed unconditionally so the direct-mode root composition
// (`root_compose::compose_root_workspace`) can return the concrete executor
// alongside its decider, the way a child run receives it.
pub(crate) use executor::ChildTools;
pub(crate) use executor::child_toolset;
pub(crate) use factory::CliChildFactory;
// `root_compose` composes the direct-mode root's workspace toolset. The
// re-export exposes the compose entrypoint and its result type to
// `agent::tools::coding::direct` and its tests.
mod root_compose;
pub(crate) use root_compose::{
    CodingOpsFactory, RootComposeRequest, RootWorkspace, compose_root_workspace,
};
// Test fixture shared by `root_compose::tests` and `coding::direct_tests`.
#[cfg(test)]
pub(crate) mod tests_root_compose_fixture;

use std::path::Path;
use std::sync::Arc;

use saya_harness::coding::worktree::{InPlaceRootInput, OwnershipInput, VerifiedCheckout};
use saya_harness::workspace::Workspace;
use saya_types::ManagedCheckoutId;

/// The proof a composition root holds for the checkout it is about to use:
/// a durable managed checkout, or an in-place folder fixed by its private
/// record. The compose paths re-verify whichever proof they are given.
pub(crate) enum CheckoutProof<'a> {
    Managed(OwnershipInput<'a>),
    InPlace(InPlaceRootInput<'a>),
}

/// The trusted pairing of the exact checkout id with the exact workspace
/// root, constructed only from a use-time `VerifiedCheckout` — never from
/// model JSON or a bare path.
pub(crate) struct TrustedCheckoutPair {
    checkout_id: ManagedCheckoutId,
    root: Arc<Workspace>,
}

impl std::fmt::Debug for TrustedCheckoutPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The root is deliberately absent from the output: the id is the only
        // identifying part of the pair.
        f.debug_struct("TrustedCheckoutPair")
            .field("checkout_id", &self.checkout_id)
            .finish_non_exhaustive()
    }
}

impl TrustedCheckoutPair {
    /// The only production constructor: pairs a use-time VerifiedCheckout's id
    /// with a Workspace opened at its path.
    pub(crate) fn from_verified(verified: &VerifiedCheckout) -> Result<Self, ChildToolsError> {
        let root = Workspace::open(Path::new(verified.path().as_str()))
            .map_err(|_| ChildToolsError::Workspace)?;
        Ok(Self {
            checkout_id: verified.checkout_id().clone(),
            root: Arc::new(root),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_tests(checkout_id: ManagedCheckoutId, root: Arc<Workspace>) -> Self {
        Self { checkout_id, root }
    }

    pub(crate) fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }
}

/// Error building or refusing the child tool surface.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub(crate) enum ChildToolsError {
    #[error("the checkout root could not be opened")]
    Workspace,
    #[error("the child authority does not match the verified checkout")]
    CheckoutMismatch,
}
