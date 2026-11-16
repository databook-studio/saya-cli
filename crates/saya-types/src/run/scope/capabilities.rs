//! The approval surface for a whole run: the scopes the user (or a headless
//! `--allow`) approved once, that every plan step is validated against.

use serde::{Deserialize, Serialize};

use super::{EndpointBindings, FetchScope};

/// The scopes a run is approved for. `Default` is "no capability at all";
/// the composition root builds an approval from CLI flags or TUI choices by
/// starting there and turning on what was approved. Fields are public so an
/// approval can be assembled, but every step is checked against the approved
/// set by [`RunPlan::validate`](crate::run::RunPlan::validate).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Capabilities {
    /// The run may write files inside its own run workspace — nowhere else.
    #[serde(rename = "write", alias = "workspace_write")]
    pub workspace_write: bool,
    /// The declared fetch destinations; `None` means no fetch capability.
    pub fetch: Option<FetchScope>,
    /// The roles bound to endpoints this run may call.
    pub endpoints: EndpointBindings,
}

impl Capabilities {
    /// True when every scope `self` asks for is approved by `approved`. A
    /// scope that names a set (destinations, bindings) must be a
    /// subset of the approved set — naming a member the run was not approved
    /// for is not a subset. The *named* difference lives in
    /// [`Capabilities::missing_from`](super::super::missing) — this rule's
    /// message, not a second comparison.
    pub fn is_subset_of(&self, approved: &Capabilities) -> bool {
        if self.workspace_write && !approved.workspace_write {
            return false;
        }
        if let Some(mine) = &self.fetch {
            let Some(theirs) = &approved.fetch else {
                return false;
            };
            if !mine
                .destinations
                .iter()
                .all(|d| theirs.destinations.contains(d))
            {
                return false;
            }
        }
        self.endpoints.is_subset_of(&approved.endpoints)
    }
}
