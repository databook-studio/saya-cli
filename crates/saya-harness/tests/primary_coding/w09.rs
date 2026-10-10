#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "w09/conflicts.rs"]
mod conflicts;
#[path = "w09/evidence.rs"]
mod evidence;

#[allow(dead_code)]
#[path = "w05/create_support.rs"]
mod create_support;

#[allow(dead_code)]
#[path = "w08/apply_support.rs"]
mod apply_support;

#[allow(dead_code)]
#[path = "w02_support.rs"]
mod support;
