//! Tests for the image outputs of a notebook read: each one is sent to the
//! model as the image itself — the image block a `read` of an image file
//! builds — and one that is not sent stands in as a placeholder line naming the
//! cell and the reason, never as its base64 bytes in the text.
//!
//! The fixtures are the notebook read's own ([`super::notebook_read`]), so the
//! two files describe the same `.ipynb` layout.

use super::*;
use crate::agent::tools::workspace::notebook::NOTEBOOK_MAX_IMAGES;
use saya_agent::IMAGE_RESULT_FIELD;
use serde_json::{Value, json};

use super::notebook_read::{PNG_SIGNATURE, base64, image_cell, seed};

#[tokio::test]
async fn read_of_a_notebook_marks_an_output_it_cannot_send_with_a_placeholder_line() {
    let sandbox = Sandbox::new("notebook-image-output");
    seed(
        &sandbox,
        "plot.ipynb",
        json!([image_cell("plot", "image/svg+xml", &base64(b"<svg/>"))]),
    );
    let tools = sandbox.tools();

    let result = tools
        .execute("read", json!({"path": "plot.ipynb"}))
        .await
        .expect("a contained notebook read must succeed");

    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert!(content.contains("output 0 [image/svg+xml]"), "{content}");
    assert!(
        content.contains("[image output: image/svg+xml, 6 bytes]"),
        "an output no provider takes as an image is a short placeholder line: {content}"
    );
    assert!(
        result.get(IMAGE_RESULT_FIELD).is_none(),
        "an output that was not sent carries no image: {result}"
    );
}

#[tokio::test]
async fn read_of_a_notebook_sends_an_image_output_as_an_image() {
    let sandbox = Sandbox::new("notebook-image-sent");
    let data = base64(PNG_SIGNATURE);
    seed(
        &sandbox,
        "plot.ipynb",
        json!([image_cell("plot", "image/png", &data)]),
    );
    let tools = sandbox.tools();

    let result = tools
        .execute("read", json!({"path": "plot.ipynb"}))
        .await
        .expect("a contained notebook read must succeed");

    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert!(
        content.contains("output 0 [image/png]"),
        "the block still names the output: {content}"
    );
    assert!(
        !content.contains(&data),
        "the base64 image data never reaches the text view: {content}"
    );
    assert_eq!(
        result[IMAGE_RESULT_FIELD],
        json!([{"media_type": "image/png", "data_base64": data}]),
        "the image rides the result out of band, as the image it is: {result}"
    );
}

#[tokio::test]
async fn read_of_a_notebook_sends_every_image_output_it_holds() {
    let sandbox = Sandbox::new("notebook-image-many");
    let png = base64(PNG_SIGNATURE);
    let jpeg = base64(b"\xff\xd8\xff\xe0");
    seed(
        &sandbox,
        "plot.ipynb",
        json!([
            image_cell("a", "image/png", &png),
            image_cell("b", "image/jpeg", &jpeg)
        ]),
    );
    let tools = sandbox.tools();

    let result = tools
        .execute("read", json!({"path": "plot.ipynb"}))
        .await
        .expect("a contained notebook read must succeed");

    assert_eq!(
        result[IMAGE_RESULT_FIELD],
        json!([
            {"media_type": "image/png", "data_base64": png},
            {"media_type": "image/jpeg", "data_base64": jpeg},
        ]),
        "one image per image output, in cell order: {result}"
    );
}

/// The media type is read from the bytes' own signature, so a notebook that
/// claims a PNG and stores something else is described as what it is not sent
/// for rather than handed to the model as a broken image.
#[tokio::test]
async fn read_of_a_notebook_does_not_send_bytes_that_are_not_that_image() {
    let sandbox = Sandbox::new("notebook-image-not-that-image");
    let data = base64(b"not a png at all");
    seed(
        &sandbox,
        "plot.ipynb",
        json!([image_cell("plot", "image/png", &data)]),
    );
    let tools = sandbox.tools();

    let result = tools
        .execute("read", json!({"path": "plot.ipynb"}))
        .await
        .expect("a contained notebook read must succeed");

    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert!(
        content.contains("not sent (cell 0, the bytes do not carry that image)"),
        "the placeholder names the cell and the reason: {content}"
    );
    assert!(result.get(IMAGE_RESULT_FIELD).is_none(), "{result}");
}

#[tokio::test]
async fn read_of_a_notebook_falls_back_to_the_placeholder_over_the_image_bound() {
    let sandbox = Sandbox::new("notebook-image-bound");
    // A real image header, then more bytes than one image may carry.
    let mut oversized = PNG_SIGNATURE.to_vec();
    oversized.resize(MAX_IMAGE_BYTES as usize + 1, 0);
    let data = base64(&oversized);
    seed(
        &sandbox,
        "plot.ipynb",
        json!([image_cell("plot", "image/png", &data)]),
    );
    let tools = sandbox.tools();

    let result = tools
        .execute("read", json!({"path": "plot.ipynb"}))
        .await
        .expect("a contained notebook read must succeed");

    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert!(
        content.contains("not sent (cell 0, over the") && content.contains("image bound)"),
        "the placeholder names the cell and the reason: {content}"
    );
    assert!(
        !content.contains(&data),
        "the over-bound image's bytes never reach the text view"
    );
    assert!(
        result.get(IMAGE_RESULT_FIELD).is_none(),
        "an image over the bound is not sent: {result}"
    );
}

#[tokio::test]
async fn read_of_a_notebook_falls_back_to_the_placeholder_past_the_image_cap() {
    let sandbox = Sandbox::new("notebook-image-cap");
    let data = base64(PNG_SIGNATURE);
    let cells: Vec<Value> = (0..=NOTEBOOK_MAX_IMAGES)
        .map(|index| image_cell(&format!("cell-{index}"), "image/png", &data))
        .collect();
    let last = NOTEBOOK_MAX_IMAGES;
    seed(&sandbox, "plot.ipynb", Value::Array(cells));
    let tools = sandbox.tools();

    let result = tools
        .execute("read", json!({"path": "plot.ipynb"}))
        .await
        .expect("a contained notebook read must succeed");

    let images = result[IMAGE_RESULT_FIELD]
        .as_array()
        .expect("the images under the cap still ride the result");
    assert_eq!(
        images.len(),
        NOTEBOOK_MAX_IMAGES,
        "one read carries at most {NOTEBOOK_MAX_IMAGES} images: {result}"
    );
    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert!(
        content.contains(&format!(
            "not sent (cell {last}, this read already carries {NOTEBOOK_MAX_IMAGES} images)"
        )),
        "the image past the cap is a placeholder naming its cell and the reason: {content}"
    );
}
