use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct Chunk {
    pub(super) choices: Vec<Choice>,
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
