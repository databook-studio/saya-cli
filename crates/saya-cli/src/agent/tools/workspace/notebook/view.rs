//! The `read` side of a notebook: the cells as text.
//!
//! One block per cell — its index, type, id when it has one, its source, and
//! each output's own text — with an image output sent as the image itself and
//! its line only describing it; an image that is not sent stands in as a short
//! placeholder line naming the cell and the reason (see [`super::images`]).
//! `content` is that text; the `size` and `digest` are still the file's own, so
//! an `edit` or `notebook_edit` precondition states a state it can check.
//!
//! Three bounds shape the view: [`NOTEBOOK_MAX_CELLS`] cells are rendered and
//! the rest are counted, not shown, one cell contributes at most
//! [`NOTEBOOK_CELL_OUTPUT_MAX_BYTES`] of output text, and one read carries at
//! most [`super::NOTEBOOK_MAX_IMAGES`] images. The response cap then bisects how many
//! of those blocks are served, exactly as the text read bisects its own
//! content — and it measures the text alone, because the images ride beside it
//! and are bounded on their own channel.

use saya_agent::{IMAGE_RESULT_FIELD, ImageBlock, ToolError, shape_tool_result};
use saya_harness::workspace::ReadFile;
use serde_json::Value;

use super::images;
use super::scan::Cells;
use super::{NOTEBOOK_CELL_OUTPUT_MAX_BYTES, NOTEBOOK_MAX_CELLS};

/// The marker closing a cell whose further output text was cut.
const MORE_OUTPUT: &str = "[more output not shown]\n";

/// The notebook read's result: the cell view, the file's own size and digest,
/// how many of the cells the view holds, and — under the reserved field the
/// loop takes out before the model sees the text — the images it carries.
pub(in crate::agent::tools) fn result(
    path: &str,
    file: ReadFile,
    cells: &Cells,
    cap: usize,
) -> Result<(Value, String, u64), ToolError> {
    let total = cells.elements.len();
    let mut blocks = Vec::with_capacity(total.min(NOTEBOOK_MAX_CELLS));
    let mut images = Vec::new();
    let mut cut = false;
    for range in cells.elements.iter().take(NOTEBOOK_MAX_CELLS) {
        let (block, block_cut) = block(&file.bytes[range.clone()], blocks.len(), &mut images);
        cut |= block_cut;
        blocks.push(block);
    }
    let projection = Projection {
        path,
        file: &file,
        blocks,
        images,
        total,
        cut,
    };
    // The largest prefix of the blocks the caller's cap can hold. The metadata
    // skeleton already fits — the read checked it before the file was read —
    // so an empty view is always admissible.
    let mut low = 0usize;
    let mut high = projection.blocks.len();
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if within_cap(&projection.json(middle), cap) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let mut result = projection.json(low);
    if !within_cap(&result, cap) {
        return Err(ToolError::NoEffect(Box::new(ToolError::Runner(
            "workspace read result exceeds caller cap".into(),
        ))));
    }
    // Added after the cap is applied: an image is measured by its own bound,
    // never by the text cap, and the loop takes it out again before anything
    // observes the result.
    let images = projection.image_blocks(low);
    if !images.is_empty() {
        result[IMAGE_RESULT_FIELD] = Value::Array(images);
    }
    Ok((result, file.digest, file.size))
}

struct Projection<'a> {
    path: &'a str,
    file: &'a ReadFile,
    blocks: Vec<String>,
    images: Vec<(usize, ImageBlock)>,
    total: usize,
    cut: bool,
}

impl Projection<'_> {
    fn json(&self, shown: usize) -> Value {
        let mut content = String::new();
        for block in &self.blocks[..shown] {
            content.push_str(block);
        }
        let remaining = self.total - shown;
        if remaining > 0 {
            content.push_str(&format!("[{remaining} more cell(s) not shown]\n"));
        }
        let mut result = serde_json::json!({
            "path": self.path,
            "size": self.file.size,
            "digest": self.file.digest,
            "content": content,
            "cells": self.total,
            "cells_shown": shown,
        });
        // Only a view that is the whole notebook leaves the flag out; then it
        // says nothing the other fields do not.
        if self.cut || remaining > 0 {
            result["truncated"] = Value::Bool(true);
        }
        result
    }

    /// The images the view carries: those of a cell it shows. The per-read cap
    /// was applied in cell order while the blocks were built and the shown
    /// cells are a prefix of them, so a placeholder line that is served always
    /// names the exact count this read carries.
    fn image_blocks(&self, shown: usize) -> Vec<Value> {
        self.images
            .iter()
            .filter(|(cell, _)| *cell < shown)
            .map(|(_, image)| {
                serde_json::json!({
                    "media_type": &image.media_type,
                    "data_base64": &image.data_base64,
                })
            })
            .collect()
    }
}

/// One cell's block: its header line, its source, and its outputs. `cut` says
/// an output's text was left out, which is what `truncated` reports.
fn block(bytes: &[u8], index: usize, images: &mut Vec<(usize, ImageBlock)>) -> (String, bool) {
    let Ok(cell) = serde_json::from_slice::<Value>(bytes) else {
        return (format!("cell {index} [unreadable]\n"), true);
    };
    let cell_type = cell
        .get("cell_type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut text = format!("cell {index} [{cell_type}]");
    if let Some(id) = cell.get("id").and_then(Value::as_str) {
        text.push_str(&format!(" id={id}"));
    }
    text.push('\n');
    text.push_str(&join_text(cell.get("source")));
    let (outputs, cut) = outputs(&cell, index, images);
    text.push_str(&outputs);
    (text, cut)
}

/// A cell's outputs as text, bounded by [`NOTEBOOK_CELL_OUTPUT_MAX_BYTES`].
fn outputs(cell: &Value, index: usize, images: &mut Vec<(usize, ImageBlock)>) -> (String, bool) {
    let Some(outputs) = cell.get("outputs").and_then(Value::as_array) else {
        return (String::new(), false);
    };
    let mut text = String::new();
    let mut cut = false;
    for (position, output) in outputs.iter().enumerate() {
        let (label, body) = output_text(output, index, images);
        let header = format!("output {position} [{label}]\n");
        let budget = NOTEBOOK_CELL_OUTPUT_MAX_BYTES.saturating_sub(text.len() + MORE_OUTPUT.len());
        if header.len() + body.len() <= budget {
            text.push_str(&header);
            text.push_str(&body);
            continue;
        }
        if budget > header.len() {
            text.push_str(&header);
            text.push_str(prefix(&body, budget - header.len()));
        }
        cut = true;
        break;
    }
    if cut {
        text.push_str(MORE_OUTPUT);
    }
    (text, cut)
}

/// One output's label and body. An image output's body is a short true line —
/// what it is, and, when it is not sent, why — and its bytes are never served
/// as text: they ride the result's own image field instead.
fn output_text(
    output: &Value,
    cell: usize,
    images: &mut Vec<(usize, ImageBlock)>,
) -> (String, String) {
    let kind = output
        .get("output_type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match kind {
        "stream" => ("stream".to_owned(), join_text(output.get("text"))),
        "error" => {
            let name = output
                .get("ename")
                .and_then(Value::as_str)
                .unwrap_or("error");
            let value = output.get("evalue").and_then(Value::as_str).unwrap_or("");
            let mut body = format!("{name}: {value}\n");
            body.push_str(&join_text(output.get("traceback")));
            ("error".to_owned(), body)
        }
        _ => {
            let Some(data) = output.get("data") else {
                return (kind.to_owned(), format!("[{kind} output not shown]\n"));
            };
            if let Some((mime, encoded)) = images::lookup(data) {
                return (mime.to_owned(), images::line(mime, encoded, cell, images));
            }
            if data.get("text/plain").is_some() {
                return ("text".to_owned(), join_text(data.get("text/plain")));
            }
            let mime = data
                .as_object()
                .and_then(|map| map.keys().next())
                .cloned()
                .unwrap_or_else(|| kind.to_owned());
            (mime.clone(), format!("[{mime} output not shown]\n"))
        }
    }
}

/// nbformat stores a cell's text either as one string or as a list of lines;
/// both are served the same way, each ending on a line boundary.
fn join_text(value: Option<&Value>) -> String {
    let mut text = match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(lines)) => lines.iter().filter_map(Value::as_str).collect::<String>(),
        _ => String::new(),
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// The longest prefix of `text` within `max` bytes, on a char boundary.
fn prefix(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn within_cap(value: &Value, cap: usize) -> bool {
    !shape_tool_result(value, cap).truncated
}
