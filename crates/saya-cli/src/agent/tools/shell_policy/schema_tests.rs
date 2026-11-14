//! The background family's advertised schema and the validation pass each of
//! its three calls shares: the shell that can start a run, and the two readers
//! that reach the runs it started.

use saya_agent::ToolError;
use serde_json::json;

use super::{arguments, schema};

fn names(definitions: &[saya_agent::ToolDefinition]) -> Vec<String> {
    definitions
        .iter()
        .map(|definition| definition.name.clone())
        .collect()
}

/// The family is three definitions advertised together: a surface that
/// advertises the start also advertises the two calls that reach the run, so
/// no model can be told to start something it cannot read or stop.
#[test]
fn the_family_is_the_shell_and_its_two_readers() {
    assert_eq!(
        names(&schema::background_definitions(
            schema::BackgroundOwner::Job
        )),
        ["shell", "shell_output", "shell_kill"]
    );
    let family = schema::background_definitions(schema::BackgroundOwner::Job);
    let properties = &family[0].parameters["properties"];
    assert_eq!(properties["run_in_background"]["type"], "boolean");
    assert_eq!(
        properties["run_in_background"]["description"],
        "Start the command and return at once with an id; you are told when it exits."
    );
    assert!(
        family[1]
            .description
            .contains("repeated reads of a quiet run wait for a change"),
        "the read states the backoff: {}",
        family[1].description
    );
    assert_eq!(family[1].parameters["required"], json!(["id"]));
    assert_eq!(
        family[1].parameters["properties"]
            .as_object()
            .expect("the read takes an object")
            .len(),
        1,
        "the read takes an id and nothing else"
    );
    assert_eq!(family[2].parameters["required"], json!(["id"]));
    assert_eq!(
        family[2].parameters["properties"]
            .as_object()
            .expect("the kill takes an object")
            .len(),
        1,
        "the kill takes an id and nothing else"
    );
}

/// The foreground shell definition every other surface advertises is
/// unchanged: a surface whose shell cannot own a background run never
/// advertises one.
#[test]
fn the_foreground_definition_has_no_background_parameter() {
    let properties = &schema::definition().parameters["properties"];
    assert!(
        properties.get("run_in_background").is_none(),
        "the foreground shell advertises no start: {properties}"
    );
    assert_eq!(
        names(&schema::background_definitions(schema::BackgroundOwner::Job)[1..]),
        ["shell_output", "shell_kill"]
    );
}

/// Each surface states the truth about what ends its runs: a helper's run
/// ends with the helper, a session's with the session, a job's with the job.
/// One builder, so the three sentences cannot drift from the registries.
#[test]
fn each_surface_states_what_ends_its_background_run() {
    for (owner, ending) in [
        (schema::BackgroundOwner::Job, "the job ends"),
        (schema::BackgroundOwner::Helper, "the helper ends"),
        (schema::BackgroundOwner::Session, "the session ends"),
    ] {
        let definition = schema::background_definitions(owner).remove(0);
        let description = definition.description;
        assert!(
            description.contains(ending),
            "{owner:?} says `{ending}`: {description}"
        );
        for other in ["the job ends", "the helper ends", "the session ends"] {
            assert!(
                other == ending || !description.contains(other),
                "{owner:?} states only its own ending: {description}"
            );
        }
    }
}

/// Reading and killing are not new authority: they reach only runs this shell
/// started, so neither asks the operator and neither claims a side effect.
#[test]
fn only_the_shell_start_carries_authority() {
    let family = schema::background_definitions(schema::BackgroundOwner::Job);
    assert!(family[0].effect.requires_approval);
    for reader in &family[1..] {
        assert!(!reader.effect.requires_approval, "{}", reader.name);
        assert!(!reader.effect.external_side_effect, "{}", reader.name);
        assert!(reader.read_only, "{}", reader.name);
    }
}

/// A timeout bounds a foreground call; a background run is ended by
/// `shell_kill` or by the job, so asking for both is refused rather than
/// silently ignored.
#[test]
fn a_background_start_that_names_a_timeout_is_refused() {
    let call = json!({"command": "true", "run_in_background": true, "timeout": 1000});
    assert!(matches!(
        arguments::validate(&call, true),
        Err(ToolError::ShellTimeoutWithBackground)
    ));
    let foreground = json!({"command": "true", "timeout": 1000});
    assert!(arguments::validate(&foreground, true).is_ok());
}

/// A shell that holds no background registry refuses the parameter outright:
/// the flag is never accepted and quietly dropped.
#[test]
fn a_shell_without_a_registry_refuses_the_background_parameter() {
    let call = json!({"command": "true", "run_in_background": true});
    assert!(matches!(
        arguments::validate(&call, false),
        Err(ToolError::UnsupportedProperty)
    ));
    assert!(arguments::validate(&call, true).is_ok());
    let not_a_flag = json!({"command": "true", "run_in_background": "yes"});
    assert!(matches!(
        arguments::validate(&not_a_flag, true),
        Err(ToolError::UnsupportedProperty)
    ));
}

/// The read takes an id and nothing else; the kill takes an id.
#[test]
fn the_readers_validate_their_own_arguments() {
    assert!(arguments::validate_output(&json!({"id": 1})).is_ok());
    assert!(arguments::validate_kill(&json!({"id": 1})).is_ok());
    for refused in [
        json!({}),
        json!({"id": 1, "wait_ms": 30000}),
        json!({"id": 1, "command": "true"}),
        json!({"id": "1"}),
    ] {
        assert!(
            matches!(
                arguments::validate_output(&refused),
                Err(ToolError::UnsupportedProperty)
            ),
            "the read must refuse {refused}"
        );
    }
    for refused in [
        json!({}),
        json!({"id": 1, "wait_ms": 10}),
        json!({"id": -1}),
    ] {
        assert!(
            matches!(
                arguments::validate_kill(&refused),
                Err(ToolError::UnsupportedProperty)
            ),
            "the kill must refuse {refused}"
        );
    }
}

/// The two readers accept no key they do not declare, exactly as the shell
/// does: an argument a call cannot honour is a refusal, never a silent drop.
#[test]
fn the_readers_take_no_undeclared_key() {
    let read = schema::background_definitions(schema::BackgroundOwner::Job)[1].clone();
    let kill = schema::background_definitions(schema::BackgroundOwner::Job)[2].clone();
    for definition in [read, kill] {
        let keys: Vec<&str> = definition.parameters["properties"]
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert!(
            keys.iter().all(|key| *key == "id"),
            "{} declares only its own keys: {keys:?}",
            definition.name
        );
    }
}

/// The advertised `id` is an integer, as every reader's validation expects.
#[test]
fn the_readers_name_an_integer_id() {
    for definition in &schema::background_definitions(schema::BackgroundOwner::Job)[1..] {
        assert_eq!(
            definition.parameters["properties"]["id"]["type"], "integer",
            "{}",
            definition.name
        );
    }
}

/// The command ceiling is 64 KiB — the harness's own number, not a second
/// copy — and a call under it is admitted while one byte over is refused with
/// the call's size and the ceiling in the message.
#[test]
fn the_command_ceiling_is_64_kib() {
    assert_eq!(schema::MAX_COMMAND_BYTES, 64 * 1024);
    assert_eq!(
        schema::MAX_COMMAND_BYTES,
        saya_harness::runner::shell::MAX_COMMAND_BYTES,
        "the advertised bound is the enforcing harness bound"
    );
    assert!(
        arguments::validate(&json!({"command": "x".repeat(60 * 1024)}), true).is_ok(),
        "a 60 KiB command is admitted"
    );
    let over = schema::MAX_COMMAND_BYTES + 1;
    let error = arguments::validate(&json!({"command": "x".repeat(over)}), true)
        .expect_err("the ceiling is a hard bound");
    let message = error.to_string();
    assert!(
        message.contains(&over.to_string()),
        "the refusal names the call's size: {message}"
    );
    assert!(
        message.contains(&schema::MAX_COMMAND_BYTES.to_string()),
        "the refusal names the ceiling: {message}"
    );
}

/// The advertised timeout ceiling is 600 s, and the description states the
/// 120 s default: a call may widen the default up to the ceiling, and one
/// above it is refused before it reaches the shell.
#[test]
fn the_timeout_ceiling_is_six_hundred_seconds() {
    assert_eq!(schema::MAX_TIMEOUT_MILLIS, 600_000);
    assert!(arguments::validate(&json!({"command": "true", "timeout": 600_000}), true).is_ok());
    assert!(matches!(
        arguments::validate(&json!({"command": "true", "timeout": 600_001}), true),
        Err(ToolError::UnsupportedProperty)
    ));
    let description = schema::definition().description;
    assert!(
        description.contains("default 120000, max 600000"),
        "the description states the default and ceiling: {description}"
    );
}
