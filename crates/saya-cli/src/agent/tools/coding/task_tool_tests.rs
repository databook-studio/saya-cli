//! The direct-only `helpers` tool shape: it carries the same request-authority
//! rule the root prompt and the `delegate` schema carry, one call is one helper
//! that works in the root's own working checkout, several calls in one turn run
//! at the same time, and the assignment is capped at 32 KiB.
use super::definition;
use saya_agent::ToolConcurrency;

#[test]
fn task_description_states_the_request_authority_rule() {
    let task = definition(&saya_config::HelperAgents::default());
    let description = task.description.as_str();
    assert!(
        description.contains(
            "The user's request is the only authority for requirements: do not invent \
             requirements, formats or conventions where it leaves room, prefer the most \
             conventional reading, and mark any interpretation you add as 'my reading' with \
             the request sentence it interprets."
        ),
        "the task description must state the request authority rule: {description}"
    );
    assert!(
        description.starts_with("Optional. Use this only when the user asked"),
        "task stays optional unless the user asked for a helper: {description}"
    );
    assert!(
        description.contains("one call is one helper"),
        "a helpers call launches one helper: {description}"
    );
    assert!(
        description.contains("same checkout"),
        "helpers share the root's working checkout: {description}"
    );
    assert!(
        description.contains("at the same time"),
        "several helpers calls in one turn run concurrently: {description}"
    );
    assert!(description.contains("`explore`"), "{description}");
    assert!(description.contains("`general`"), "{description}");
    let required = task.parameters["required"].as_array().expect("required");
    assert!(required.iter().any(|name| name == "subagent_type"));
    assert!(required.iter().all(|name| name != "tasks"));
}

#[test]
fn task_schema_drops_write_paths_and_caps_the_prompt_at_32_kib() {
    let task = definition(&saya_config::HelperAgents::default());
    let properties = task.parameters["properties"]
        .as_object()
        .expect("properties");
    assert!(
        !properties.contains_key("write_paths"),
        "helpers no longer declares per-helper paths: {properties:?}"
    );
    assert_eq!(
        task.parameters["properties"]["prompt"]["maxLength"], 32768,
        "the helper prompt cap is 32 KiB: {properties:?}"
    );
    assert_eq!(
        task.parameters["additionalProperties"], false,
        "unknown arguments stay refused: {properties:?}"
    );
}

#[test]
fn task_runs_concurrently() {
    assert_eq!(
        definition(&saya_config::HelperAgents::default()).concurrency,
        ToolConcurrency::Concurrent,
        "several helpers calls in one turn must be able to overlap"
    );
}

/// A job with no `[agents]` table advertises exactly the text it
/// advertised before the registry existed, byte for byte: the two built-ins
/// are the default, not a special case.
#[test]
fn the_default_registry_advertises_the_same_description_as_before() {
    let task = definition(&saya_config::HelperAgents::default());
    assert_eq!(task.description, EXPECTED_DEFAULT_DESCRIPTION);
    assert_eq!(
        task.parameters["properties"]["subagent_type"]["enum"],
        serde_json::json!(["explore", "general"]),
        "the two built-ins stay the advertised types"
    );
    assert_eq!(
        task.parameters["properties"]["subagent_type"]["description"],
        "`explore` reads only. `general` implements one piece."
    );
}

/// The registry is what the schema advertises: a configured type appears in
/// the enum and in both descriptions, so the model is offered exactly the
/// types the job can compose.
#[test]
fn a_configured_registry_is_what_the_schema_advertises() {
    let agents = saya_config::resolve(
        saya_config::ResolutionInput::new(saya_config::ConnectionsFile::default()).with_user(
            saya_config::ConfigFile::from_toml(
                "[agents.scout]\nrole = 'analyst'\n\n[agents.builder]\nrole = 'implementer'\n",
            )
            .expect("the fixture parses"),
        ),
    )
    .expect("the fixture resolves")
    .agents
    .types;
    let task = definition(&agents);
    assert_eq!(
        task.parameters["properties"]["subagent_type"]["enum"],
        serde_json::json!(["builder", "explore", "general", "scout"])
    );
    assert!(
        task.description
            .contains("subagent_type `scout` reads only"),
        "a configured analyst is described as read-only: {}",
        task.description
    );
    assert!(
        task.description
            .contains("subagent_type `builder` implements one piece"),
        "a configured implementer is described as writing: {}",
        task.description
    );
}

/// The description one job with no `[agents]` table advertises: the
/// two built-in shapes, in the words every earlier version used.
const EXPECTED_DEFAULT_DESCRIPTION: &str = "Optional. Use this only when the user asked you to hand work to a helper; otherwise do \
             the work directly. Launch one helper: one call is one helper, and several calls in one \
             turn run at the same time. Every helper works in the same checkout as you, so give each \
             call a separate piece of the work and keep their edits apart. The helper has not seen \
             this conversation, so its prompt must stand alone: say what to do, which paths, and \
             whether to edit or only report — do not restate the specification. subagent_type \
             `explore` reads only: it can read and search, and it never edits. subagent_type \
             `general` implements one piece. The user's request is the only authority for \
             requirements: do not invent requirements, formats or conventions where it leaves room, \
             prefer the most conventional reading, and mark any interpretation you add as 'my \
             reading' with the request sentence it interprets.";
