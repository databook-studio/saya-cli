//! Preferences store: scope, isolation, replace, unset, migration, and the
//! admission gate.
//!
//! A preference is *not a claim*: no object, no fingerprint, no drift, no
//! lifecycle, no evidence. These tests cover the store half of the slice —
//! the typed values are unit-tested in `saya-types`.

use saya_types::ProfileIdentity;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "preferences/admission.rs"]
mod admission;
#[path = "preferences/roundtrip.rs"]
mod roundtrip;
#[path = "preferences/scope.rs"]
mod scope;
#[path = "preferences/serde_gate.rs"]
mod serde_gate;

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-pref-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile_a() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}
fn profile_b() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "b".repeat(64))).unwrap()
}
