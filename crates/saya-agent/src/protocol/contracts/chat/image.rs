//! The image a message carries beside its text.
//!
//! An image is not JSON text: it cannot ride the `content` string the loop
//! measures, replays, and redacts. It rides the message instead, already
//! base64-encoded so every adapter sends the same bytes, and it is bounded on
//! its own channel (the workspace read's image cap) rather than by the
//! context byte budget.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The reserved key a tool result uses to hand an image to the loop. The
/// workspace `read` tool writes it; the loop takes it out before the result is
/// observed, shaped, or recorded, so image bytes never reach the model as text
/// and never reach the session transcript at all.
pub const IMAGE_RESULT_FIELD: &str = "image";

/// One image a message carries to a provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageBlock {
    /// The media type of `data_base64`, e.g. `image/png`.
    pub media_type: String,
    /// The image's own bytes in standard base64, with no line breaks.
    pub data_base64: String,
}

/// Takes the images out of a tool result, leaving the result text-only.
///
/// A result carries one image or a list of them: a workspace `read` of an image
/// file carries one, and a `read` of a notebook carries one per image output.
/// Only those tools write the field, and they write it from typed values, so a
/// field that does not parse cannot come from them: it is dropped rather than
/// rendered as a broken image, and one unparsable entry does not cost the
/// result the images that do parse. Nothing is dropped silently on the adapter
/// side — an adapter that cannot send an image says so in the result's text.
pub fn take_image_blocks(result: &mut Value) -> Vec<ImageBlock> {
    let Some(value) = result
        .as_object_mut()
        .and_then(|object| object.remove(IMAGE_RESULT_FIELD))
    else {
        return Vec::new();
    };
    match value {
        Value::Array(values) => values.into_iter().filter_map(parse).collect(),
        value => parse(value).into_iter().collect(),
    }
}

/// One image of the field, or nothing when it is not a whole image.
fn parse(value: Value) -> Option<ImageBlock> {
    let image = serde_json::from_value::<ImageBlock>(value).ok()?;
    (!image.media_type.is_empty() && !image.data_base64.is_empty()).then_some(image)
}
