#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "w08/apply.rs"]
mod apply;
#[allow(dead_code)]
#[path = "w08/apply_support.rs"]
mod apply_support;
#[path = "w08/candidate.rs"]
mod candidate;
#[path = "w08/support.rs"]
mod support_active;

#[allow(dead_code)]
#[path = "w05/create_support.rs"]
mod create_support;

#[allow(dead_code)]
#[path = "w02_support.rs"]
mod support;
