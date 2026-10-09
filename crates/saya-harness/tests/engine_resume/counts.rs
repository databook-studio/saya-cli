//! The count evidence a public resume carries, and the legacy count
//! budget's refusal. The doubles (`stubs`) and the family rig (`rig`)
//! live beside the case batteries (`carry_through`, `legacy_refusal`).

use super::*;

#[path = "counts/stubs.rs"]
mod stubs;

#[path = "counts/rig.rs"]
mod rig;

#[path = "counts/carry_through.rs"]
mod carry_through;

#[path = "counts/legacy_refusal.rs"]
mod legacy_refusal;
