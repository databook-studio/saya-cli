//! Native application owner contracts; runtime execution awaits final gates.
#[path = "primary_coding/approval.rs"]
mod approval;
#[path = "primary_coding/audit.rs"]
mod audit;
#[cfg(unix)]
#[path = "primary_coding/checkouts.rs"]
mod checkouts;
#[cfg(unix)]
#[path = "primary_coding/doctor.rs"]
mod doctor;
#[cfg(unix)]
#[path = "primary_coding/operations.rs"]
mod operations;
#[path = "primary_coding/parity.rs"]
mod parity;
#[cfg(unix)]
#[path = "primary_coding/promotion.rs"]
mod promotion;
#[cfg(unix)]
#[path = "primary_coding/reads.rs"]
mod reads;
#[path = "primary_coding/reconcile.rs"]
mod reconcile;
#[cfg(unix)]
#[path = "primary_coding/review.rs"]
mod review;
#[cfg(unix)]
#[path = "primary_coding/support.rs"]
#[allow(dead_code)] // shared test harness: this file uses only part of it
mod support;
#[cfg(unix)]
#[path = "primary_coding/surfaces.rs"]
mod surfaces;
#[path = "primary_coding/u07.rs"]
mod u07;
#[cfg(unix)]
#[path = "primary_coding/x02.rs"]
mod x02;
