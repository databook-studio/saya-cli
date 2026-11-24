//! The root's database lane: the plugin's approval table on the run where the
//! plugin serves the chat surface, and nothing at all on a job surface.
//!
//! The lane is attached by the run's composition, never by a tool: a run that
//! put the database plugin on the chat surface hands the root one, and every
//! other run — `-p`, a background start, a helper — composes none and keeps the
//! decider its surface always had.

use std::sync::Arc;

use saya_agent::ToolDefinition;
use serde_json::Value;

use crate::plugins::database;
use crate::plugins::database::approval::DbApprovalLane;

use super::direct::RootTools;

impl RootTools {
    /// Attaches the plugin's database lane. Called once, by the composition
    /// that states this run's surface; a root that never gets one decides its
    /// database tools exactly as it did before the lane existed.
    pub(crate) fn attach_db_lane(&mut self, lane: Arc<DbApprovalLane>) {
        match self {
            RootTools::ReadOnly(read_only) => read_only.db_lane = Some(lane),
            RootTools::WorkingCheckout(direct) => direct.db_lane = Some(lane),
        }
    }

    /// The lane this run took, when it took one.
    pub(super) fn db_lane(&self) -> Option<&DbApprovalLane> {
        match self {
            RootTools::ReadOnly(read_only) => read_only.db_lane.as_deref(),
            RootTools::WorkingCheckout(direct) => direct.db_lane.as_deref(),
        }
    }

    /// The lane's answer for this call, or `None` when this run took no lane or
    /// the name is not one the database plugin routes — the two cases the
    /// caller decides as it always did.
    pub(super) async fn db_lane_approve(
        &self,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> Option<bool> {
        let lane = self.db_lane().filter(|_| database::routes(&tool.name))?;
        Some(lane.approve(tool, arguments).await)
    }

    /// [`Self::db_lane_approve`]'s answer without asking, so a caller that
    /// must know whether a question is coming — the hook gate that may
    /// pre-empt one — reads the same table the ask would.
    pub(super) fn db_lane_would_ask(
        &self,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> Option<bool> {
        let lane = self.db_lane().filter(|_| database::routes(&tool.name))?;
        Some(lane.would_ask(tool, arguments))
    }
}
