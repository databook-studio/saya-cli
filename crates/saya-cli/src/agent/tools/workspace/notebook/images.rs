//! The image outputs of a notebook read.
//!
//! An image output is sent to the model as the image it is — the same image
//! block a `read` of an image file builds — so its base64 never reaches
//! `content` as text. What is sent is what the file itself says: the media type
//! is read from the decoded bytes' own signature, never from the key the
//! notebook claims, and one image is bounded by the same
//! [`MAX_IMAGE_BYTES`] an image file read is.
//!
//! An output that is not sent stands in as a placeholder line naming the cell
//! and the reason, so the model reads that an image exists and why it did not
//! arrive rather than finding it silently missing. `image/svg+xml` is not a
//! refusal: no provider takes an SVG as an image block, so it keeps the plain
//! line it always had.

use base64::Engine as _;
use saya_agent::ImageBlock;
use serde_json::Value;

use super::super::read::{MAX_IMAGE_BYTES, media_type};
use super::NOTEBOOK_MAX_IMAGES;

/// The media types a provider takes as an image block, in the order they are
/// looked for when an output's `data` carries more than one.
const SENDABLE_MIMES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// The other media types an output may carry that are an image to look at but
/// not an image block to send.
const OTHER_IMAGE_MIMES: &[&str] = &["image/svg+xml"];

/// The image an output's `data` carries, as its declared media type and the
/// base64 the notebook stores it as.
pub(super) fn lookup(data: &Value) -> Option<(&'static str, &str)> {
    SENDABLE_MIMES
        .iter()
        .chain(OTHER_IMAGE_MIMES)
        .find_map(|mime| Some((*mime, data.get(*mime)?.as_str()?)))
}

/// One image output's line, and the image itself when it rides the result.
pub(super) fn line(
    mime: &'static str,
    encoded: &str,
    cell: usize,
    images: &mut Vec<(usize, ImageBlock)>,
) -> String {
    let bytes = decoded_len(encoded);
    let described = |body: &str| format!("[image output{body}: {mime}, {bytes} bytes]\n");
    match ride(mime, encoded, bytes, images.len()) {
        Ride::Sent => {
            images.push((
                cell,
                ImageBlock {
                    media_type: mime.to_owned(),
                    data_base64: encoded.to_owned(),
                },
            ));
            described("")
        }
        Ride::Plain => described(""),
        Ride::Refused(reason) => described(&format!(" not sent (cell {cell}, {reason})")),
    }
}

enum Ride {
    /// Sent as an image, so the line only describes it.
    Sent,
    /// An image to look at, but not one any provider takes as an image block.
    Plain,
    /// Not sent, for this reason.
    Refused(String),
}

/// Whether this image output is sent, and when it is not, why. The reasons are
/// checked in the order they can be known, and each is a fact about this output
/// or this read — never a guess.
fn ride(mime: &str, encoded: &str, bytes: usize, carried: usize) -> Ride {
    if !SENDABLE_MIMES.contains(&mime) {
        return Ride::Plain;
    }
    if bytes > MAX_IMAGE_BYTES as usize {
        return Ride::Refused(format!("over the {MAX_IMAGE_BYTES}-byte image bound"));
    }
    if !carries(mime, encoded) {
        return Ride::Refused("the bytes do not carry that image".to_owned());
    }
    if carried >= NOTEBOOK_MAX_IMAGES {
        return Ride::Refused(format!(
            "this read already carries {NOTEBOOK_MAX_IMAGES} images"
        ));
    }
    Ride::Sent
}

/// Whether `encoded` really carries the image `mime` claims, read from the
/// decoded bytes' own signature — the same check a `read` of an image file
/// makes, so the media type the model is handed is true. Only the first sixteen
/// base64 characters are decoded: a signature is at most twelve bytes long, and
/// the rest of a multi-megabyte image is never touched.
fn carries(mime: &str, encoded: &str) -> bool {
    let bytes = encoded.as_bytes();
    let end = bytes.len().min(16);
    let head = &bytes[..end - end % 4];
    base64::engine::general_purpose::STANDARD
        .decode(head)
        .is_ok_and(|decoded| media_type(&decoded) == Some(mime))
}

/// An image's byte count from its base64 length, so an image is measured
/// without being decoded.
fn decoded_len(encoded: &str) -> usize {
    let padding = encoded
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    (encoded.len() / 4 * 3).saturating_sub(padding)
}
