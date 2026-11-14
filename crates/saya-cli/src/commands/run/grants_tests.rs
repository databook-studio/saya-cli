//! The resume-authority pair: the
//! grant a resume carries is derived from the journal's `PlanApproved`
//! payload — never from `spec.json`, which anything with the run directory's
//! permissions could rewrite between invocations.

use std::fs;
use std::path::PathBuf;

use saya_harness::journal::Journal;
use saya_types::{Capabilities, Destination, FetchScope, RunEvent};

#[path = "grants_tests/journal_authority.rs"]
mod journal_authority;
#[path = "grants_tests/seed_words.rs"]
mod seed_words;
#[path = "grants_tests/unstated_approvals.rs"]
mod unstated_approvals;

pub(super) use super::grants::journal_grants;

fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-run-grants-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn journal_with(scopes: Vec<String>, dir: &PathBuf) {
    Journal::open(dir)
        .append(&RunEvent::PlanApproved {
            scopes: Some(scopes),
        })
        .expect("the approval is journaled");
}

fn spec_scopes(destinations: &[&str]) -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    if !destinations.is_empty() {
        capabilities.fetch = Some(
            FetchScope::new(
                destinations
                    .iter()
                    .map(|host| Destination::new("https", *host).expect("a shape-valid host"))
                    .collect(),
            )
            .expect("the spec's fetch scope parses"),
        );
    }
    capabilities
}
