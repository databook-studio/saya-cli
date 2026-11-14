//! Coordinator tool routing for the direct-mode root: which tool names the
//! coordinator executor handles. `helpers` is the direct-only synchronous
//! delegation tool; it is a coordinator route, advertised solely by
//! `DirectCodingTools`.

/// Coordinator tool names routed to the coordinator executor. Database tools
/// are workspace names (the workspace owns them) and route to the workspace.
/// No supervisor tools exist; `helpers` is the whole delegation surface.
pub(super) const COORDINATOR_ROUTES: &[&str] = &["helpers"];

pub(super) fn routes_to_coordinator(name: &str) -> bool {
    COORDINATOR_ROUTES.contains(&name)
}

/// Names the direct root owns itself: coordinator routes plus the job-local
/// `checklist`. Child agents do not see the coordinator routes; the workspace
/// decider never classifies them.
pub(super) fn routes_to_root_owned(name: &str) -> bool {
    name == "checklist" || routes_to_coordinator(name)
}
