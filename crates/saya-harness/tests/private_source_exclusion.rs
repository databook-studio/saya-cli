#[path = "coding_runtime_contracts/private_file_exclusions.rs"]
mod source_filtering;
#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "coding_runtime_contracts/worktree_inventory_support.rs"]
mod support;
