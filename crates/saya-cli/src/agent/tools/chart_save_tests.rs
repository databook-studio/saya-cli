//! Tests for the chart save's guarded publication (`chart_save`): the publish
//! step runs against a real temp workspace, and the tool-level tests cover
//! only refusal outcomes — every refusal returns before the private temp copy
//! and the browser open, so no test here ever opens a browser.

use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{ToolError, ToolExecutor};
use saya_harness::workspace::{MAX_IO_BYTES, Workspace};

use super::database_tools::chart_save::publish_chart_html;

/// A sandbox workspace under the OS temp dir, removed on drop.
struct SaveWorkspace {
    root: PathBuf,
    workspace: Workspace,
}

impl SaveWorkspace {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("saya-chart-publish-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).expect("sandbox directory must create");
        let workspace = Workspace::open(&root).expect("workspace root must open");
        Self { root, workspace }
    }

    fn seed(&self, rel: &str, bytes: &[u8]) {
        self.workspace
            .create_new(rel, bytes)
            .expect("seed file must create");
    }

    fn observed_digest(&self, rel: &str) -> String {
        self.workspace
            .read(rel, MAX_IO_BYTES as u64)
            .expect("the file must read back")
            .digest
    }
}

impl Drop for SaveWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// -- the publish step against a real workspace --------------------------------

#[path = "chart_save_tests/publish.rs"]
mod publish;
#[path = "chart_save_tests/render.rs"]
mod render;
// -- argument typing -----------------------------------------------------------

// -- the advertised schema ------------------------------------------------------

// -- through the real tool (refusals only: they return before any open) ---------
#[path = "chart_save_tests/run_step.rs"]
mod run_step;
