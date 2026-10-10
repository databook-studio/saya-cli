//! The child tool surface: workspace tools bound to one verified managed checkout.
#![allow(dead_code)] // consumed by R05b2

mod executor;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_edit;
#[cfg(test)]
mod tests_runner;

pub(crate) use executor::{ChildTools, child_toolset, child_toolset_with_runner};

use std::path::Path;
use std::sync::Arc;

use saya_harness::coding::worktree::VerifiedCheckout;
use saya_harness::workspace::Workspace;
use saya_types::ManagedCheckoutId;

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
    #[error("the child authority does not admit the runner")]
    RunnerNotAdmitted,
}
