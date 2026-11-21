//! The write admission every mutating door runs before the anchored mutation:
//! the owner must still be alive, the view must permit workspace writes, and
//! the path must be one the contained checkout can hold. A session's held
//! grant for the workspace-write family pre-answers the path's shape and the
//! authority's answer; the workspace tool's own containment is then the last
//! word, and it refuses an escaping path with its typed error.

use saya_agent::ToolError;
use saya_types::WorkspacePath;

use super::super::ChildTools;

impl ChildTools {
    pub(super) fn admit_mutation(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if self
            .coding
            .as_ref()
            .is_some_and(|coding| coding.failed.load(std::sync::atomic::Ordering::Acquire))
        {
            return Err(ToolError::NoEffect(Box::new(ToolError::Runner(
                "the native job owner failed and cannot admit another workspace mutation".into(),
            ))));
        }
        if !self.permit_writes {
            return Err(ToolError::NoEffect(Box::new(ToolError::UnsupportedTool)));
        }
        let path = arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ToolError::NoEffect(Box::new(ToolError::PathNotString)))?;
        if path.len() > 512 {
            return Err(ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                "the path exceeds the child scope bound".into(),
            ))));
        }
        if !self.granted() {
            WorkspacePath::try_from(path.to_owned()).map_err(|_| {
                ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                    "the path is not an admissible workspace path".into(),
                )))
            })?;
        }
        if let Some(scope) = &self.write_scope {
            // The `--write` scope is a path ceiling over workspace paths. A
            // path that is not a workspace path at all is not the scope's to
            // refuse when the session holds the family's grant: the workspace
            // tool's containment answers it, with its own typed error. Every
            // other session is refused here, as before.
            let outside = match WorkspacePath::try_from(path.to_owned()) {
                Ok(path) => !scope.contains(path.as_str()),
                Err(_) => !self.granted(),
            };
            if outside {
                return Err(ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                    super::super::scope::outside_write_scope(scope),
                ))));
            }
        }
        if !self.granted() {
            self.view
                .admit_workspace_write(&self.checkout_id)
                .map_err(|_| {
                    ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                        "the child authority does not admit this write".into(),
                    )))
                })?;
        }
        Ok(())
    }
}
