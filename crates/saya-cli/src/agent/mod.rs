// `Decision.fanout` is part of the decision contract (exercised by the decide
// tests) but not yet consumed by the production caller; the rest of the module
// is wired in via `candidates`.
pub(crate) mod candidates;
#[allow(dead_code)]
pub(crate) mod decide;
pub(crate) mod extraction_trace;
pub(crate) mod knowledge_event;
pub(crate) mod learning;
pub(crate) mod profile;
pub(crate) mod provider;
pub(crate) mod recall_context;
pub(crate) mod runtime;
pub(crate) mod state_tools;
pub(crate) mod system_prompt;
pub(crate) mod tools;
pub(crate) mod turn_config;
pub(crate) mod turn_inputs;
