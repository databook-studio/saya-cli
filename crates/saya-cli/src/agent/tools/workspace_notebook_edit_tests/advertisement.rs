//! Where the tool is advertised at all. A surface that cannot write the
//! workspace is offered no `notebook_edit` — hidden, not merely refused, so
//! the model is never invited to call it.

use super::*;

#[test]
fn notebook_edit_is_advertised_only_where_workspace_writes_are_permitted() {
    let with_writes: Vec<String> = DatabaseTools::definitions(false, false, false, true)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(
        with_writes.contains(&"notebook_edit".to_string()),
        "a surface that permits workspace writes advertises notebook_edit: {with_writes:?}"
    );
    let without: Vec<String> = DatabaseTools::definitions(false, false, false, false)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(
        !without.contains(&"notebook_edit".to_string()),
        "a read-only surface advertises no notebook_edit: {without:?}"
    );
}

/// The tool list is a cached prefix. `notebook_edit` is appended last, so
/// `read`, `list`, `glob`, `grep`, `edit` and `write` keep their positions and
/// the prefix the provider caches is unchanged.
#[test]
fn notebook_edit_is_appended_after_the_tools_that_existed_before_it() {
    let names: Vec<String> = DatabaseTools::definitions(false, false, false, true)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert_eq!(
        names,
        [
            "schema_discovery",
            "request_clarification",
            "read",
            "list",
            "glob",
            "grep",
            "edit",
            "write",
            "notebook_edit",
        ],
        "notebook_edit goes last, and nothing before it moved"
    );
}

/// The advertised text and schema are bounded and complete. The description
/// rides every request of every run, so it states only what a correct call
/// needs; the schema keeps every field, type, enum and required list, so a
/// trimmed wording can never quietly drop one.
#[test]
fn notebook_edit_advertises_a_bounded_description_and_a_complete_schema() {
    let definition = DatabaseTools::definitions(false, false, false, true)
        .into_iter()
        .find(|tool| tool.name == "notebook_edit")
        .expect("a surface that permits workspace writes advertises notebook_edit");
    assert!(
        definition.description.chars().count() <= 350,
        "the description is bounded: {} chars",
        definition.description.chars().count()
    );
    let parameters = &definition.parameters;
    assert_eq!(parameters["required"], serde_json::json!(["path", "mode"]));
    assert_eq!(parameters["additionalProperties"], serde_json::json!(false));
    let properties = parameters["properties"].as_object().expect("an object");
    let mut fields: Vec<&str> = properties.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        [
            "cell_id",
            "cell_type",
            "expected_digest",
            "index",
            "mode",
            "path",
            "source",
        ],
        "every field survives the trim"
    );
    assert_eq!(
        properties["mode"]["enum"],
        serde_json::json!(["replace", "insert", "delete"])
    );
    assert_eq!(
        properties["cell_type"]["enum"],
        serde_json::json!(["code", "markdown"])
    );
    assert_eq!(properties["index"]["type"], serde_json::json!("integer"));
    assert_eq!(properties["index"]["minimum"], serde_json::json!(0));
    assert_eq!(properties["path"]["type"], serde_json::json!("string"));
    assert_eq!(properties["source"]["type"], serde_json::json!("string"));
    assert_eq!(
        properties["expected_digest"]["type"],
        serde_json::json!("string")
    );
}
