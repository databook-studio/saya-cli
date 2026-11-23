//! Tests for the one place a surface applies `[tools] allow` / `--tools`:
//! the advertisement filter.

use saya_agent::{LocalStateEffect, ToolDefinition, ToolEffect};
use saya_config::{ResolvedTools, ToolsAllow};

use super::narrow;

fn definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: String::new(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: Default::default(),
        completion: None,
    }
}

fn stated(names: &[&str]) -> ResolvedTools {
    ResolvedTools {
        allow: Some(ToolsAllow::parse(names).unwrap()),
        ..ResolvedTools::default()
    }
}

#[test]
fn an_unstated_list_keeps_every_definition() {
    let mut definitions = vec![definition("read"), definition("write"), definition("shell")];
    narrow(&mut definitions, &ResolvedTools::default());
    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["read", "write", "shell"]);
}

#[test]
fn a_stated_list_keeps_only_its_names_in_composition_order() {
    let mut definitions = vec![definition("read"), definition("write"), definition("shell")];
    narrow(&mut definitions, &stated(&["shell", "read"]));
    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["read", "shell"]);
}

#[test]
fn a_name_the_surface_never_composed_is_not_added() {
    let mut definitions = vec![definition("read")];
    narrow(&mut definitions, &stated(&["read", "write"]));
    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["read"]);
}

#[test]
fn the_bash_spelling_survives_a_list_that_names_shell() {
    // The model may say `bash`; the canonical name is `shell`. A list that
    // names the tool keeps whichever spelling the surface advertised.
    let mut definitions = vec![definition("shell"), definition("bash"), definition("read")];
    narrow(&mut definitions, &stated(&["shell"]));
    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["shell", "bash"]);
}

#[test]
fn a_connection_tied_tool_survives_any_list() {
    // The list governs the tools a surface composes for itself. The
    // connection-tied ones follow the selected connection, so a list can
    // neither remove one here nor name one at all.
    let mut definitions = vec![definition("read"), definition("schema_discovery")];
    narrow(&mut definitions, &stated(&["read"]));
    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["read", "schema_discovery"]);
}

#[test]
fn a_database_tool_cannot_be_named_at_all() {
    // The connection-tied tools follow the selected connection, so the list
    // refuses the name rather than silently doing nothing.
    assert!(ToolsAllow::parse(["schema_discovery"]).is_err());
}
