//! F4b: the data-file route names the `shell` tool, so a job without a shell
//! must not carry it. `DuckdbAvailable` is the detected state of a folder that
//! holds a data file while `duckdb` is on `PATH`; the gate is the same
//! `file_sources::for_session` the interactive session uses.

use super::system_with_file_sources;
use crate::agent::file_sources::{FileSources, guidance};

/// A read-only job's root toolset never offers `shell`, so the folder that
/// gives a write job the route gives this job none.
#[test]
fn a_read_only_job_in_a_data_folder_gets_no_data_file_block() {
    let prompt = system_with_file_sources(false, true, FileSources::DuckdbAvailable);
    assert!(
        !prompt.contains("duckdb"),
        "a job with no shell must not be told to use it: {prompt}"
    );
}

/// The write job in the same folder still gets the route, and it stays the
/// last thing in the prompt so the cacheable prefix is unchanged.
#[test]
fn a_write_job_in_the_same_folder_gets_the_block_last() {
    let detected = FileSources::DuckdbAvailable;
    let prompt = system_with_file_sources(false, false, detected);
    let block = guidance(detected).expect("the data-file route is offered");
    assert!(prompt.ends_with(block), "the block is last: {prompt}");
}
