//! The list's own vocabulary: how one word in `[tools] allow` maps onto the
//! capability it names.
//!
//! The shell's two readers are parts of the one `shell`, the catalog's two
//! lists do not overlap, and the MCP family — whose names come from
//! third-party servers and so cannot be catalogued — is governed by shape.

use saya_config::{ResolvedTools, ToolsAllow, ToolsAllowError, resolve};

use super::with_user;

/// The shell's two readers are parts of the one `shell` capability. A list
/// that names `shell` keeps the whole family — a start whose reader the list
/// removed would be a promise nothing could keep — a list that names a reader
/// states `shell`, and a list without `shell` keeps none of the three.
#[test]
fn the_shell_family_is_one_name_in_the_list() {
    let stated =
        ResolvedTools::from_stated(Some(&["shell".into(), "read".into()])).expect("a valid list");
    for name in ["shell", "shell_output", "shell_kill"] {
        assert!(stated.admits(name), "`shell` keeps {name}");
    }

    let reader_only =
        ResolvedTools::from_stated(Some(&["shell_output".into()])).expect("a valid list");
    assert!(
        reader_only.admits("shell_kill"),
        "a reader states the shell"
    );
    assert_eq!(reader_only.stated(), Some(vec!["shell".to_string()]));

    let without = ResolvedTools::from_stated(Some(&["helpers".into(), "write".into()]))
        .expect("a valid list");
    for name in ["shell", "shell_output", "shell_kill"] {
        assert!(!without.admits(name), "a list without `shell` drops {name}");
    }
}

/// The catalog is the one vocabulary, and the two lists do not overlap: a name
/// is either a capability the list may take away or a connection-tied tool it
/// must not name.
#[test]
fn the_two_name_lists_are_disjoint_and_exhaustive_of_their_own_meaning() {
    for name in saya_config::GOVERNED_TOOLS {
        assert!(
            saya_config::is_governed_tool(name),
            "{name} is governed by construction"
        );
        assert!(
            !saya_config::is_database_tool(name),
            "{name} must not also be connection-tied"
        );
    }
    for name in saya_config::DATABASE_TOOLS {
        assert!(
            saya_config::is_database_tool(name),
            "{name} is connection-tied by construction"
        );
        assert!(
            !saya_config::is_governed_tool(name),
            "{name} must not also be governed"
        );
    }
}

/// An MCP tool name is governed by shape, not by the catalog: the names come
/// from third-party servers the configuration names, so no fixed list can hold
/// them. Both spellings the operator may write are governed names.
#[test]
fn mcp_names_are_governed() {
    assert!(saya_config::is_governed_tool("mcp__files__read_file"));
    assert!(saya_config::is_governed_tool("mcp__files"));
    assert!(
        !saya_config::is_database_tool("mcp__files__read_file"),
        "an MCP tool is a capability, not a connection-tied tool"
    );
}

/// The server's whole entry admits every tool of that server, including tools
/// the server had not advertised when the operator wrote the list. The exact
/// entry admits only the tool it names.
#[test]
fn a_server_entry_admits_every_tool_of_that_server() {
    let whole = ResolvedTools::from_stated(Some(&["mcp__files".into(), "read".into()]))
        .expect("a valid list");
    assert!(whole.admits("mcp__files__read_file"));
    assert!(whole.admits("mcp__files__a_tool_added_later"));
    assert!(
        !whole.admits("mcp__other__read_file"),
        "one server's entry admits no other server's tools"
    );

    let exact = ResolvedTools::from_stated(Some(&["mcp__files__read_file".into(), "read".into()]))
        .expect("a valid list");
    assert!(exact.admits("mcp__files__read_file"));
    assert!(
        !exact.admits("mcp__files__write_file"),
        "the exact entry admits exactly the tool it names"
    );
}

/// A name that reaches for the MCP namespace and misses is refused with the
/// shape's own reason: the fix is the spelling, not a different tool.
#[test]
fn a_malformed_mcp_name_is_refused_by_shape() {
    for word in [
        "mcp__Files__read", // an uppercase server name
        "mcp__a__b",        // fine — the separator rule is on the *server* name
        "mcp__",            // no server at all
        "mcp__files__",     // no tool after the separator
        "mcp__files__a b",  // a tool name a provider cannot carry
    ] {
        let expected = !word.starts_with("mcp__a__b");
        let outcome = ToolsAllow::parse([word]);
        assert_eq!(
            outcome.is_ok(),
            !expected,
            "{word:?} must {}: {outcome:?}",
            if expected { "be refused" } else { "resolve" }
        );
        if expected {
            assert!(
                matches!(outcome, Err(ToolsAllowError::MalformedMcpName { .. })),
                "{word:?} must be refused by shape, got {outcome:?}"
            );
        }
    }
}

/// A tool segment may hold the separator: only the *server* name may not, so
/// `mcp__a__b__c` reads one way and one way only — server `a`, tool `b__c`.
#[test]
fn a_tool_segment_may_hold_the_separator() {
    assert_eq!(saya_config::mcp_server_of("mcp__a__b__c"), Some("mcp__a"));
    assert_eq!(saya_config::mcp_server_of("mcp__files"), Some("mcp__files"));
    assert_eq!(saya_config::mcp_server_of("read"), None);
    assert_eq!(saya_config::mcp_server_of("mcp__"), None);
}

/// The `[tools] allow` list is a narrowing list, so a name it admits that no
/// configured server produces is a name that simply removes nothing: the
/// catalog cannot know which servers are configured, and refusing an unstated
/// one would make a config's validity depend on another table's contents.
#[test]
fn an_mcp_name_resolves_from_a_config_table() {
    let resolved = resolve(with_user("[tools]\nallow = [\"read\", \"mcp__files\"]\n"))
        .expect("a valid list resolves");
    let allow = resolved.tools.allow.clone().expect("the list was stated");
    assert_eq!(allow.names().collect::<Vec<_>>(), ["mcp__files", "read"]);
    assert!(resolved.tools.admits("mcp__files__read_file"));
}
