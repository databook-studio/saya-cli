use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct Chunk {
    #[serde(default)]
    pub(super) message: Option<Message>,
    #[serde(default)]
    pub(super) done: bool,
    /// Prompt token count from the final done record.
    #[serde(default)]
    pub(super) prompt_eval_count: Option<u64>,
    /// Generated token count from the final done record.
    #[serde(default)]
    pub(super) eval_count: Option<u64>,
}
#[derive(Deserialize)]
pub(super) struct Message {
    #[serde(default)]
    pub(super) content: String,
    #[serde(default)]
    pub(super) tool_calls: Vec<Call>,
}
#[derive(Deserialize)]
pub(super) struct Call {
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) function: Function,
}
#[derive(Deserialize)]
pub(super) struct Function {
    pub(super) name: String,
    pub(super) arguments: serde_json::Value,
}
