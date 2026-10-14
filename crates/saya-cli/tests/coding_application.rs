//! Native application owner contracts; runtime execution awaits final gates.
#[path = "coding_application/approval.rs"]
mod approval;
#[path = "coding_application/audit.rs"]
mod audit;
#[cfg(unix)]
#[path = "coding_application/checkouts.rs"]
mod checkouts;
#[cfg(unix)]
#[path = "coding_application/doctor.rs"]
mod doctor;
#[path = "coding_application/fresh_home.rs"]
mod fresh_home;
#[cfg(unix)]
#[path = "coding_application/mcp_extension_boundary.rs"]
mod mcp_extension_boundary;
#[cfg(unix)]
#[path = "coding_application/operations.rs"]
mod operations;
#[path = "coding_application/parity.rs"]
mod parity;
#[cfg(unix)]
#[path = "coding_application/promotion.rs"]
mod promotion;
#[cfg(unix)]
#[path = "coding_application/reads.rs"]
mod reads;
#[path = "coding_application/reconcile.rs"]
mod reconcile;
#[path = "coding_application/reconcile_bounds.rs"]
mod reconcile_bounds;
#[cfg(unix)]
#[path = "coding_application/review.rs"]
mod review;
#[path = "coding_application/setup_readiness_report.rs"]
mod setup_readiness_report;
#[cfg(unix)]
#[path = "coding_application/support.rs"]
#[allow(dead_code)] // shared test harness: this file uses only part of it
mod support;
#[cfg(unix)]
#[path = "coding_application/surfaces.rs"]
mod surfaces;
