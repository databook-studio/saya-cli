//! How an adapter carries the images on a message — or says it cannot.
//!
//! Two adapters can carry one: Anthropic puts the image in the tool result's
//! own content blocks, and the OpenAI-compatible wire puts it in a following
//! `user` message. Gemini's `functionResponse` and Ollama's message content are
//! JSON strings, so neither can: they append [`unavailable_note`] to the
//! result's text instead, and the model learns the image exists and did not
//! arrive rather than reading a silent drop.

use crate::ImageBlock;

/// The note an adapter that cannot send images appends to a result's text.
/// Short and true, and it never carries the image's bytes.
pub(super) fn unavailable_note(images: &[ImageBlock]) -> String {
    match images {
        [image] => format!(
            "\n\n[saya: the image in this result ({}) could not be sent to this provider]",
            image.media_type
        ),
        images => format!(
            "\n\n[saya: {} images in this result could not be sent to this provider]",
            images.len()
        ),
    }
}

/// `content` with the unavailable note appended when `images` cannot ride it,
/// and `content` unchanged when there is nothing to report.
pub(super) fn text_with_note(content: &str, images: &[ImageBlock]) -> String {
    if images.is_empty() {
        return content.to_owned();
    }
    format!("{content}{}", unavailable_note(images))
}

/// The data URL the OpenAI-compatible wire carries an image as.
pub(super) fn data_url(image: &ImageBlock) -> String {
    format!("data:{};base64,{}", image.media_type, image.data_base64)
}

#[cfg(test)]
mod tests {
    use super::{ImageBlock, unavailable_note};

    fn image(media_type: &str) -> ImageBlock {
        ImageBlock {
            media_type: media_type.into(),
            data_base64: "AAAA".into(),
        }
    }

    /// A notebook read carries one image per image output, so an adapter that
    /// cannot send them says how many did not arrive — never just one.
    #[test]
    fn the_note_names_the_count_when_a_result_carries_several_images() {
        assert_eq!(
            unavailable_note(&[image("image/png")]),
            "\n\n[saya: the image in this result (image/png) could not be sent to this provider]"
        );
        assert_eq!(
            unavailable_note(&[image("image/png"), image("image/jpeg")]),
            "\n\n[saya: 2 images in this result could not be sent to this provider]"
        );
    }
}
