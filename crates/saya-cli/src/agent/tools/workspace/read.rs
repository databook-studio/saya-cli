//! Bounded contained file reads and their model-facing response bound.

use saya_agent::{ToolError, ToolExecutionContext};

use super::super::DatabaseTools;

mod execute;

// The signature check is one rule for the whole workspace read surface: a file
// read and a notebook's image output both send an image only when its bytes
// carry the image its media type claims.
pub(in crate::agent::tools) use execute::image::media_type;

/// The file bytes one `read` retains per call. The model-facing result is
/// still bisected to the loop's message cap, so this bounds the read, not
/// what the model receives.
pub(crate) const WORKSPACE_READ_MAX_BYTES: u64 = 256 * 1024;

/// The largest image one `read` will carry, in the file's own bytes. An image
/// is never truncated to fit: it is refused whole, naming this bound and the
/// file's size. Base64 adds a third on the wire, so this keeps the encoded
/// image under 4 MiB — comfortably inside what the image-carrying providers
/// accept — while staying well inside the 8 MiB read bound.
pub(crate) const MAX_IMAGE_BYTES: u64 = 3 * 1024 * 1024;

/// The widest line window one ranged read selects. It bounds the *selection*,
/// not the response: the model-facing byte cap still bisects what is served,
/// and `selected_range_complete` says whether the window fit.
pub(crate) const WORKSPACE_READ_MAX_LINES: u64 = 2_000;

impl DatabaseTools {
    pub(super) async fn workspace_read(
        &self,
        arguments: &serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        execute::execute(self, arguments, context).await
    }
}
