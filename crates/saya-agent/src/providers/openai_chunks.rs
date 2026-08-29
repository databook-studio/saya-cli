use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct Chunk {
    pub(super) choices: Vec<Choice>,
    #[serde(default)]
    pub(super) usage: Option<Usage>,
}
#[derive(Deserialize)]
pub(super) struct Choice {
    pub(super) delta: Delta,
    #[serde(default)]
    pub(super) finish_reason: Option<String>,
}
#[derive(Deserialize, Default)]
pub(super) struct Delta {
    #[serde(default)]
    pub(super) content: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Vec<Call>,
}
#[derive(Deserialize)]
pub(super) struct Call {
    pub(super) index: usize,
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) function: Function,
}
#[derive(Deserialize, Default)]
pub(super) struct Function {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) arguments: Option<String>,
}

/// Token counts from the trailing usage-only chunk requested via
/// `stream_options.include_usage`.
#[derive(Deserialize, Default)]
pub(super) struct Usage {
    #[serde(default, alias = "input_tokens")]
    pub(super) prompt_tokens: Option<u64>,
    #[serde(default, alias = "output_tokens")]
    pub(super) completion_tokens: Option<u64>,
}
