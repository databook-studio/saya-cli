//! H0 — the host executor's named properties. Pure addition: nothing in
//! `runner` is touched, and these tests run on every unix CI host (macOS and
//! Linux); the two group-kill tests gate on unix process groups and compile
//! to nothing elsewhere.

#[cfg(unix)]
#[path = "host/host_battery.rs"]
mod host_battery;
