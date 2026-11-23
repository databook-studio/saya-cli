//! The host a line session's run is composed over, and the shape of the folder
//! it opens over.
//!
//! The folder's shape is read first, because it decides which root the
//! application opens over: a folder that cannot root an in-place job — the home
//! directory, a dangerous root, a folder that would hold the job's private state
//! root — is a folder no run has a workspace in, so the run is composed there
//! without one rather than refused before its first turn.
//!
//! **A session that bound no folder has no workspace either.** The trust
//! question answered `n`, a launch from the OS temporary directory: there is no
//! folder a run could root, so the run is composed with no workspace at all —
//! never over the directory the process happens to stand in, whose own shape
//! would hand it the file, shell and write tools the operator declined.
//!
//! **One statement of the rule.** [`WorkspaceShape::workspace`] answers whether
//! a run over the folder has a workspace at all, and this file — the composition
//! every line surface passes through, the plain loop's, a resumed session's and
//! the one-turn `ask`'s — is the one place that reads it. A surface bound to a
//! `--workspace` — or to the folder the trust question answered `y` for — opens
//! over that root, whose own shape is `Writable`, so the rule leaves it exactly
//! as it was.

use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::CancellationToken;

use super::open::LineSession;
use crate::chat::{Workspace, WorkspaceShape, workspace_shape};
use crate::commands::coding::CodingApplication;
use crate::config::runtime::RuntimeConfig;

/// Why a run over a session that bound no folder has no workspace: a run's
/// workspace is the folder the session bound, and it bound none.
pub(super) const NO_FOLDER: &str = "the session bound no workspace";

/// The host the session's run is composed over, and the shape its folder has.
pub(crate) async fn open_host(
    parts: &LineSession,
) -> Result<(Arc<CodingApplication>, WorkspaceShape), String> {
    let config: &RuntimeConfig = &parts.runtime;
    let token = CancellationToken::new();
    // Where this session's records live, and what its folder may be: both are
    // read before the application opens, because the shape decides which root
    // the application opens over.
    let state_root = state_root(parts)?;
    let shape = match &parts.folder {
        Some(folder) => workspace_shape(folder, &state_root, &parts.session),
        None => WorkspaceShape::NoWorkspace {
            reason: NO_FOLDER.to_owned(),
        },
    };
    let application = match shape.workspace() {
        Workspace::None => {
            // The folder a run with no workspace carries is a label and nothing
            // else: the session's own when it bound one, otherwise the record
            // root the session owns. Nothing reads it — no file, shell or write
            // tool is composed over it.
            let label = parts.folder.as_deref().unwrap_or(&state_root);
            CodingApplication::session_without_workspace(
                config.clone(),
                parts.store.clone(),
                label,
                crate::cli::RootMode::NoWorkspace,
                &state_root,
                &token,
            )
            .await
        }
        Workspace::Here => {
            let folder = parts
                .folder
                .as_deref()
                .expect("a run with a workspace is composed over the folder it bound");
            match &parts.state_root {
                Some(root) => {
                    CodingApplication::session_over(
                        config.clone(),
                        parts.store.clone(),
                        folder,
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
                        folder,
                        crate::cli::RootMode::InPlace,
                        &token,
                    )
                    .await
                }
            }
        }
    }
    .map_err(|error| error.to_string())?;
    // The run this session composes is the session's own: its folder's lease
    // names the session, so a job the session opens over the same folder is
    // admitted beside it instead of refused.
    application.set_session_parent(Some(&parts.session));
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
