use super::coding_checkpoint::{self, Observation};
use crate::ImageBlock;
use serde_json::Value;

pub(super) struct ToolOutcome {
    pub(super) result: Value,
    pub(super) summary: String,
    pub(super) succeeded: bool,
    pub(super) effect_free: bool,
    pub(super) observation: Observation,
    /// Images the result carried out of band. Empty for every outcome that
    /// proved nothing but text, which is all of them but a media read.
    pub(super) images: Vec<ImageBlock>,
}

impl ToolOutcome {
    pub(super) fn refused(result: Value, summary: String) -> Self {
        Self {
            result,
            summary,
            succeeded: false,
            effect_free: false,
            observation: Observation::default(),
            images: Vec::new(),
        }
    }
    pub(super) fn completed(
        name: &str,
        arguments: &Value,
        mut result: Value,
        summary: String,
    ) -> Self {
        // The image leaves the result here, before anything else sees it: the
        // transcript, the model-facing text, the context measurement, and the
        // checkpoint observation all read `result`, and none of them may carry
        // the bytes.
        let images = crate::protocol::contracts::take_image_blocks(&mut result);
        let observation = coding_checkpoint::observe(name, arguments, Ok(&result));
        Self {
            result,
            summary,
            succeeded: true,
            effect_free: false,
            observation,
            images,
        }
    }
    pub(super) fn failed(
        name: &str,
        arguments: &Value,
        error: &crate::ToolError,
        summary: String,
    ) -> Self {
        Self {
            result: serde_json::json!({"error":error.to_string()}),
            summary,
            succeeded: false,
            effect_free: error.proves_no_effect(),
            observation: coding_checkpoint::observe(name, arguments, Err(error)),
            images: Vec::new(),
        }
    }
}
