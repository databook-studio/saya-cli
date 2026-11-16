//! Tests for the image handoff out of a tool result: the bytes leave the
//! result before anything observes it — so they never reach the model as text,
//! the transcript, or the context measurement — and ride the message instead.

use super::{ToolOutcome, complete_tool};
use crate::{IMAGE_RESULT_FIELD, ImageBlock};
use serde_json::{Value, json};

const DATA: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// The result the workspace `read` returns for an image: a short true text
/// content, and the image itself under the reserved field.
fn image_result() -> Value {
    let mut result = json!({
        "path": "assets/pixel.png",
        "size": 70,
        "digest": "a".repeat(64),
        "content": "image/png, 70 bytes",
    });
    result[IMAGE_RESULT_FIELD] = json!({"media_type": "image/png", "data_base64": DATA});
    result
}

#[test]
fn a_result_image_leaves_the_result_and_rides_the_message() {
    let arguments = json!({"path": "assets/pixel.png"});
    let outcome = ToolOutcome::completed("read", &arguments, image_result(), "read".into());

    assert_eq!(
        outcome.images,
        vec![ImageBlock {
            media_type: "image/png".into(),
            data_base64: DATA.into(),
        }],
        "the image is taken out of the result and carried by the outcome"
    );
    assert!(
        outcome.result.get(IMAGE_RESULT_FIELD).is_none(),
        "the bytes must not survive in the result any consumer reads: {}",
        outcome.result
    );

    let completed = complete_tool("c1".into(), outcome, true, 65_536);
    assert_eq!(completed.message.images.len(), 1);
    assert!(
        !completed.message.content.contains(DATA),
        "the image must not be base64 in the model-facing text: {}",
        completed.message.content
    );
    assert!(
        completed.message.content.contains("image/png, 70 bytes"),
        "the text still names what was read: {}",
        completed.message.content
    );
    assert!(
        !completed.result.to_string().contains(DATA),
        "the recorded result stays text-only: {}",
        completed.result
    );
}

#[test]
fn a_result_without_an_image_is_unchanged() {
    let arguments = json!({"path": "notes.md"});
    let result = json!({
        "path": "notes.md",
        "size": 5,
        "digest": "b".repeat(64),
        "content": "hello",
    });
    let outcome = ToolOutcome::completed("read", &arguments, result, "read".into());
    assert!(outcome.images.is_empty());

    let completed = complete_tool("c2".into(), outcome, true, 65_536);
    assert!(completed.message.images.is_empty());
    assert!(completed.message.content.contains("hello"));
}

/// Only the read tool writes the reserved field, from typed values, so this
/// cannot arise from it — and if it somehow does, it must not become a broken
/// image block or a text view of its bytes.
#[test]
fn a_malformed_image_field_is_not_carried_as_an_image() {
    let mut result = json!({"path": "a.png", "content": "image/png, 4 bytes"});
    result[IMAGE_RESULT_FIELD] = json!({"media_type": "image/png"});
    let arguments = json!({"path": "a.png"});

    let outcome = ToolOutcome::completed("read", &arguments, result, "read".into());
    assert!(outcome.images.is_empty());
    assert!(outcome.result.get(IMAGE_RESULT_FIELD).is_none());
}

/// A notebook read carries one image per image output, so the reserved field
/// takes the images as a list as well as one alone.
#[test]
fn a_result_carrying_several_images_rides_them_all() {
    let mut result = json!({"path": "analysis.ipynb", "content": "cell 0 [code]"});
    result[IMAGE_RESULT_FIELD] = json!([
        {"media_type": "image/png", "data_base64": DATA},
        {"media_type": "image/jpeg", "data_base64": "AAAA"},
    ]);
    let arguments = json!({"path": "analysis.ipynb"});

    let outcome = ToolOutcome::completed("read", &arguments, result, "read".into());
    assert_eq!(
        outcome.images,
        vec![
            ImageBlock {
                media_type: "image/png".into(),
                data_base64: DATA.into(),
            },
            ImageBlock {
                media_type: "image/jpeg".into(),
                data_base64: "AAAA".into(),
            },
        ],
        "every image the result carried is taken out and kept in order"
    );
    assert!(outcome.result.get(IMAGE_RESULT_FIELD).is_none());

    let completed = complete_tool("c3".into(), outcome, true, 65_536);
    assert_eq!(completed.message.images.len(), 2);
    assert!(
        !completed.message.content.contains(DATA),
        "no image is base64 in the model-facing text: {}",
        completed.message.content
    );
    assert!(
        !completed.result.to_string().contains(DATA),
        "the recorded result stays text-only: {}",
        completed.result
    );
}

/// One entry that does not parse does not cost the result the images that do.
#[test]
fn an_unparsable_image_in_a_list_is_dropped_and_the_rest_ride() {
    let mut result = json!({"path": "analysis.ipynb", "content": "cell 0 [code]"});
    result[IMAGE_RESULT_FIELD] = json!([
        {"media_type": "image/png"},
        {"media_type": "image/png", "data_base64": DATA},
    ]);
    let arguments = json!({"path": "analysis.ipynb"});

    let outcome = ToolOutcome::completed("read", &arguments, result, "read".into());
    assert_eq!(
        outcome.images,
        vec![ImageBlock {
            media_type: "image/png".into(),
            data_base64: DATA.into(),
        }]
    );
    assert!(outcome.result.get(IMAGE_RESULT_FIELD).is_none());
}
