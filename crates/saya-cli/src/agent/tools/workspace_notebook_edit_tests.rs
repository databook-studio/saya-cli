//! Tests for the `notebook_edit` tool: one cell of a `.ipynb` is replaced,
//! inserted or deleted by index or id, and the file that lands is valid
//! nbformat JSON whose untouched cells keep their exact bytes. Like `edit` the
//! tool needs the digest the model observed, and a file that moved under it is
//! refused with nothing written.
//!
//! The fixtures below are shared by the three test files: what each mode
//! writes and leaves alone, what each refusal refuses, and where the tool is
//! advertised at all.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_harness::workspace::Workspace;

#[path = "workspace_notebook_edit_tests/advertisement.rs"]
mod advertisement;
#[path = "workspace_notebook_edit_tests/modes.rs"]
mod modes;
#[path = "workspace_notebook_edit_tests/refusals.rs"]
mod refusals;

/// A notebook whose bytes are written by hand, so a test can assert that an
/// untouched cell survives byte-for-byte: `KEEP` is a cell spelled the way
/// nothing in this codebase would spell it.
const NOTEBOOK: &str = r##"{
 "cells": [
  {"cell_type": "markdown", "id": "intro", "metadata": {}, "source": ["# Title"]},
  {"cell_type": "code", "id": "code-1", "execution_count": 1, "metadata": {}, "outputs": [{"name": "stdout", "output_type": "stream", "text": ["1\n"]}], "source": ["print(1)"]},
  {"metadata":{},"cell_type":"markdown","source":["keep me exactly"],"id":"keep"}
 ],
 "metadata": {"language_info": {"name": "python"}},
 "nbformat": 4,
 "nbformat_minor": 5
}
"##;

/// The third cell's own bytes: an edit that does not touch it must leave this
/// substring exactly where it was.
const KEEP: &str =
    r#"{"metadata":{},"cell_type":"markdown","source":["keep me exactly"],"id":"keep"}"#;

struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-nbedit-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }

    fn seed(&self, path: &str, bytes: &str) {
        self.ws
            .write(path, bytes.as_bytes())
            .expect("contained write must succeed");
    }

    fn digest(&self, path: &str) -> String {
        self.ws
            .read(path, 0)
            .expect("the seeded file must read")
            .digest
    }

    fn text(&self, path: &str) -> String {
        fs::read_to_string(self.outer.join("ws").join(path)).expect("the file must read back")
    }

    fn json(&self, path: &str) -> serde_json::Value {
        serde_json::from_str(&self.text(path)).expect("the file must still be valid JSON")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

fn cell_source(cell: &serde_json::Value) -> String {
    match &cell["source"] {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(lines) => lines
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<String>(),
        other => panic!("a cell's source is text or lines, never {other}"),
    }
}
