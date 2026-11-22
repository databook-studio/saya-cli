//! The host a line session's run is composed over, and the shape of the folder
//! it opens over.
//!
//! The folder's shape is read first, because it decides which root the
//! application opens over: a folder that cannot root an in-place job — the home
//! directory, a dangerous root, a folder that would hold the job's private state
//! root — is a folder no run has a workspace in, so the run is composed there
//! without one rather than refused before its first turn.
//!
//! **One statement of the rule.** [`WorkspaceShape::workspace`] answers whether
//! a run over the folder has a workspace at all, and this file — the composition
//! every line surface passes through, the plain loop's, a resumed session's and
//! the one-turn `ask`'s — is the one place that reads it. A surface bound to a
//! `--workspace` opens over that root, whose own shape is `Writable`, so the rule
//! leaves it exactly as it was.

use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::CancellationToken;

use super::open::LineSession;
use crate::chat::{Workspace, WorkspaceShape, workspace_shape};
use crate::commands::coding::CodingApplication;
use crate::config::runtime::RuntimeConfig;

/// The host the session's run is composed over, and the shape its folder has.
pub(super) async fn open_host(
    parts: &LineSession,
) -> Result<(Arc<CodingApplication>, WorkspaceShape), String> {
    let config: &RuntimeConfig = &parts.runtime;
    let token = CancellationToken::new();
    // Where this session's records live, and what its folder may be: both are
    // read before the application opens, because the shape decides which root
    // the application opens over.
    let state_root = state_root(parts)?;
    let shape = workspace_shape(&parts.folder, &state_root);
    let application = match shape.workspace() {
        Workspace::None => {
            CodingApplication::session_without_workspace(
                config.clone(),
                parts.store.clone(),
                &parts.folder,
                crate::cli::RootMode::NoWorkspace,
                &state_root,
                &token,
            )
            .await
        }
        Workspace::Here => match &parts.state_root {
            Some(root) => {
                CodingApplication::session_over(
                    config.clone(),
                    parts.store.clone(),
                    &parts.folder,
                    crate::cli::RootMode::InPlace,
                    root,
                    &token,
                )
                .await
            }
            None => {
                CodingApplication::session(
                    config.clone(),
                    parts.store.clone(),
                    &parts.folder,
                    crate::cli::RootMode::InPlace,
                    &token,
                )
                .await
            }
        },
    }
    .map_err(|error| error.to_string())?;
    Ok((application, shape))
}

/// Where this session's records live: the surface's own root when it stated
/// one, and the configured root otherwise.
pub(super) fn state_root(parts: &LineSession) -> Result<PathBuf, String> {
    match parts.state_root.clone() {
        Some(root) => Ok(root),
        None => coding_state_root(&parts.runtime),
    }
}

/// The coding state root the session's runs record themselves under — the same
/// root a `saya run` over this folder uses, read from the one resolver so a
/// folder's lease is looked for where it is held.
pub(super) fn coding_state_root(config: &RuntimeConfig) -> Result<PathBuf, String> {
    crate::commands::coding::root_mode::session_state_root(config)
        .map_err(|error| error.to_string())
}
