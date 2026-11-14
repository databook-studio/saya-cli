//! `[mcp_servers]` merging: one server's whole entry at a time.
//!
//! A layer states a server's whole entry or none of it. A server is a program
//! and the arguments and environment it runs with, so a field-wise merge would
//! be able to leave a higher layer's `command` beside a lower layer's `args` —
//! a program running with words written for another. Stating an entry replaces
//! that entry; servers the layer does not mention are left as they were.
//!
//! The whole table is protected (see [`crate::layers`]), so in practice the
//! only layers that reach here are trusted ones: the user's config file, an
//! explicit `--config`, and the environment.

use crate::provenance::{Recorder, SettingSource};

use super::McpServersFile;

/// Merges one layer's `[mcp_servers]` over the base, entry by entry.
pub(crate) fn merge(
    base: &mut McpServersFile,
    layer: &McpServersFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    for (name, entry) in &layer.servers {
        base.servers.insert(name.clone(), entry.clone());
        // The key names the server, so a diagnostic says which entry moved
        // rather than that the table did.
        recorder.record(&format!("mcp_servers.{name}"), source(name));
    }
}
