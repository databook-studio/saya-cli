// `Decision.fanout` is part of the decision contract (exercised by the decide
// tests) but not yet consumed by the production caller; the rest of the module
// is wired in via `candidates`.
pub(crate) mod candidates;
pub(crate) mod context;
#[allow(dead_code)]
pub(crate) mod decide;
pub(crate) mod extraction_trace;
pub(crate) mod file_sources;
pub(crate) mod hooks;
pub(crate) mod knowledge_event;
pub(crate) mod learning;
pub(crate) mod limits;
pub(crate) mod preflight_config;
pub(crate) mod profile;
pub(crate) mod provider;
// The old surface was this seam's only caller; X1 deleted it. The code here
// is X2/X3's to remove, kept compiled rather than deleted twice.
#[allow(dead_code)]
pub(crate) mod question_scope;
pub(crate) mod recall_context;
pub(crate) mod recall_turn;
#[allow(dead_code)]
// mechanism only — not yet wired into the agent loop; a later change decides when it runs
pub(crate) mod roundtrip;
pub(crate) mod runtime;
// The session wiring for checkpoint recovery: the one line the
// per-turn `DatabaseTools` build gains.
pub(crate) mod session_facts;
pub(crate) mod session_recovery_attach;
pub(crate) mod state_tools;
pub(crate) mod system_prompt;
pub(crate) mod tools;
pub(crate) mod turn_config;
pub(crate) mod turn_context;
pub(crate) mod turn_inputs;
