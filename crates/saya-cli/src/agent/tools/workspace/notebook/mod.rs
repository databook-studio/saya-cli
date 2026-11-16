//! Notebooks: `.ipynb` files, served and edited as the cells they are.
//!
//! A notebook is JSON, so it needs no new dependency — but it is *not* served
//! as JSON. `read` of a `.ipynb` renders its cells as text (index, type, id,
//! source, outputs; each image output sent as the image itself, one that is not
//! sent as a short placeholder line), and `notebook_edit` replaces, inserts or
//! deletes exactly one cell. Both are bounded the same way every other
//! workspace tool is: the file bytes by [`NOTEBOOK_MAX_BYTES`] on the read side
//! and by [`super::edit::WORKSPACE_EDIT_MAX_FILE_BYTES`] on the edit side, the
//! cells shown per read by [`NOTEBOOK_MAX_CELLS`], the images one read carries
//! by [`NOTEBOOK_MAX_IMAGES`], one image by
//! [`super::read::MAX_IMAGE_BYTES`], one cell's output text by
//! [`NOTEBOOK_CELL_OUTPUT_MAX_BYTES`], and one edit's source by
//! [`super::edit::WORKSPACE_EDIT_MAX_BYTES`].
//!
//! `notebook_edit` is `edit`'s write path, not a second one: the same
//! contained workspace, the same digest precondition, the same checkpoint
//! capture, the same redaction-placeholder guard, the same guarded commit.
//! What is its own is that it splices the file's own bytes rather than
//! re-serializing it, so a cell it does not touch keeps its exact bytes — see
//! [`scan`]. It is also the one tool offered on a property of the tree rather
//! than of the composition: a surface advertises it only where the workspace
//! actually holds a notebook — see [`presence`].

use saya_agent::ToolError;
use serde_json::Value;

mod args;
mod cells;
mod edit;
mod images;
mod presence;
mod scan;
mod splice;
mod view;

pub(crate) use presence::{drop_notebook_edit, present};
pub(in crate::agent::tools) use scan::scan;
pub(in crate::agent::tools) use view::result;

/// The most cells one notebook read renders. Beyond this the rest are counted
/// in `[N more cell(s) not shown]` and named by the `cells` field, so a
/// thousand-cell notebook cannot spend the whole context on cell bodies.
pub(in crate::agent::tools) const NOTEBOOK_MAX_CELLS: usize = 200;

/// The most output text one cell contributes to a read, in bytes. A cell that
/// streams for an hour is one cell; this is what keeps it from being the whole
/// response.
pub(in crate::agent::tools) const NOTEBOOK_CELL_OUTPUT_MAX_BYTES: usize = 8 * 1024;

/// The most image outputs one notebook read sends as images. Beyond this the
/// rest stand in as a placeholder line naming the cell and the reason, so a
/// notebook of a hundred plots cannot spend the whole request on images.
pub(in crate::agent::tools) const NOTEBOOK_MAX_IMAGES: usize = 8;

/// The most notebook bytes one read retains, in the file's own bytes. A
/// notebook is parsed whole, and one embedded image pushes it far past the text
/// read's bound — so this is the same bound `notebook_edit` will write the file
/// back under, and the same one the ranged read already holds.
pub(in crate::agent::tools) const NOTEBOOK_MAX_BYTES: u64 =
    super::edit::WORKSPACE_EDIT_MAX_FILE_BYTES;

/// Whether `path` names a notebook by its extension. The bytes still decide:
/// a `.ipynb` that is not a notebook keeps the ordinary text path.
pub(in crate::agent::tools) fn is_candidate(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("ipynb"))
}

/// The same predicate `validation.rs` and the dispatch read, so an argument
/// the tool would refuse is refused before the call is routed.
pub(in crate::agent::tools) fn validate_arguments(arguments: &Value) -> Result<(), ToolError> {
    args::parse_arguments(arguments).map(|_| ())
}
