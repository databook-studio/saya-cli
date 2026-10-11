#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "w11b/admission.rs"]
mod admission;
#[allow(dead_code)]
#[path = "w08/apply_support.rs"]
mod apply_support;
#[path = "w11b/binding.rs"]
mod binding;
#[allow(dead_code)]
#[path = "w05/create_support.rs"]
mod create_support;
#[path = "w11b/identity.rs"]
mod identity;
#[path = "w11b/recovery.rs"]
mod recovery;
#[allow(dead_code)]
#[path = "w02_support.rs"]
mod support;
#[allow(dead_code)]
#[path = "w11/support.rs"]
mod support_w11;
