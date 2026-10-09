//! The `scratch_sql` tool: the run's one writable SQL surface (ADR 0003).
//!
//! Admitted only when the run's approved capabilities include the `scratch`
//! scope — hidden-not-advertised, the same pattern the other run tools use:
//! without the scope the tool is never constructed and never advertised, so
//! nothing is denied at call time that was offered at call time. It is not a
//! `DatabaseConnector` and never enters the `ConnectionRegistry`: a scratch
//! write has no type-level path to any user database.

mod definitions;
mod execute;
mod import_call;

use std::{path::Path, sync::Arc, time::Duration};

use saya_types::Capabilities;

use super::ScratchError;
use super::open::ScratchDb;
use crate::workspace::Workspace;

/// The tool's name in the run engine's toolset.
pub const SCRATCH_SQL_TOOL: &str = "scratch_sql";
/// The contained CSV staging tool's name.
pub const SCRATCH_IMPORT_TOOL: &str = "scratch_import";

/// The admitted `scratch_sql` tool over one run's opened scratch database.
#[derive(Clone)]
pub struct ScratchSql {
    db: ScratchDb,
    workspace: Option<Arc<Workspace>>,
}

impl ScratchSql {
    /// Admits the tool only when the run's capabilities include the `scratch`
    /// scope. A refused admission opens nothing — the scratch file is not
    /// touched — and returns `None` for the run toolset to simply not
    /// advertise.
    pub fn admit(
        run_root: &Path,
        capabilities: &Capabilities,
    ) -> Result<Option<Self>, ScratchError> {
        if !capabilities.scratch {
            return Ok(None);
        }
        Ok(Some(Self {
            db: ScratchDb::open(run_root)?,
            workspace: None,
        }))
    }

    /// Opens the tool over a root the caller has already gated: the
    /// interactive session's `sessions/<id>/`, where admission is per call
    /// (the approval engine asks; nothing is pre-declared to admit). The
    /// file, configuration, and 0600 hardening are the pinned scratch
    /// semantics either way.
    pub fn open(root: &Path) -> Result<Self, ScratchError> {
        Ok(Self {
            db: ScratchDb::open(root)?,
            workspace: None,
        })
    }

    /// Narrows the per-statement timeout, for the run engine to follow a
    /// step's remaining budget. Never called with a longer value.
    pub fn with_query_timeout(mut self, query_timeout: Duration) -> Self {
        self.db = self.db.with_query_timeout(query_timeout);
        self
    }

    /// Binds the one workspace whose contained CSV files may be staged into
    /// this scratch database. Without it, import is hidden and refuses.
    pub fn with_workspace(mut self, workspace: Arc<Workspace>) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Rebinds the workspace import root for a recomposed session. `None`
    /// clears a prior binding, so an unbound session cannot retain imports.
    pub fn rebind_workspace(mut self, workspace: Option<Arc<Workspace>>) -> Self {
        self.workspace = workspace;
        self
    }
}
