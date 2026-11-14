//! The result of one create run and its redacted presentation.

use std::fmt;

use crate::coding::snapshot::MaterializedSnapshot;
use crate::coding::worktree::VerifiedCheckout;
use saya_types::{ManagedCheckoutId, ManagedCheckoutPath};

/// The result of one create run. A `receipt` of `None` means the checkout
/// was already `Active` (idempotent replay) and nothing was re-created.
pub struct CreatedCheckout {
    checkout_id: ManagedCheckoutId,
    path: ManagedCheckoutPath,
    receipt: Option<MaterializedSnapshot>,
    /// The proof a completed fresh create can hand to the same start flow, so
    /// the workspace composition and the first readiness view do not re-walk
    /// the checkout the flow just created. `None` on replay (which has not
    /// read the recorded plan bytes) and when the proof could not be built.
    verified: Option<VerifiedCheckout>,
}

impl CreatedCheckout {
    pub(super) fn completed(
        checkout_id: ManagedCheckoutId,
        path: ManagedCheckoutPath,
        receipt: MaterializedSnapshot,
        verified: Option<VerifiedCheckout>,
    ) -> Self {
        Self {
            checkout_id,
            path,
            receipt: Some(receipt),
            verified,
        }
    }

    pub(super) fn replayed(checkout_id: ManagedCheckoutId, path: ManagedCheckoutPath) -> Self {
        Self {
            checkout_id,
            path,
            receipt: None,
            verified: None,
        }
    }

    /// The completed create's use-time proof, for the caller that composes the
    /// workspace over this checkout moments later.
    pub fn into_verified(self) -> Option<VerifiedCheckout> {
        self.verified
    }

    pub fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }

    pub fn path(&self) -> &ManagedCheckoutPath {
        &self.path
    }

    pub fn receipt(&self) -> Option<&MaterializedSnapshot> {
        self.receipt.as_ref()
    }
}

/// Redacted `Debug`: the checkout id and whether a receipt was produced —
/// never the managed path.
impl fmt::Debug for CreatedCheckout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedCheckout")
            .field("checkout_id", &self.checkout_id)
            .field("has_receipt", &self.receipt.is_some())
            .field("has_verified", &self.verified.is_some())
            .finish_non_exhaustive()
    }
}
