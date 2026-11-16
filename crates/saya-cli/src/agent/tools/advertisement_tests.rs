//! The evidence rule itself: which tools the lean default set suppresses, and
//! what turns each one back on. The surfaces' own tests prove they apply it.

use saya_config::ResolvedTools;

use super::advertisement::{Evidence, advertised};

fn stated(names: &[&str]) -> ResolvedTools {
    ResolvedTools::from_stated(Some(
        &names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>(),
    ))
    .expect("the names are governed tools")
}

#[test]
fn an_unstated_list_advertises_the_lean_set_and_suppresses_the_rest() {
    let evidence = Evidence::new("", false);
    let tools = ResolvedTools::default();
    for name in [
        "read",
        "write",
        "edit",
        "glob",
        "grep",
        "shell",
        "checklist",
        "notebook_edit",
        "submit_plan",
        "shell_output",
        "shell_kill",
        "schema_discovery",
    ] {
        assert!(advertised(&evidence, &tools, name), "{name} is advertised");
    }
    for name in ["list", "delete", "web_fetch", "web_search", "helpers"] {
        assert!(!advertised(&evidence, &tools, name), "{name} is suppressed");
    }
}

#[test]
fn a_url_in_the_request_advertises_the_web_pair() {
    for request in [
        "fetch https://docs.example.org/a",
        "read HTTP://example.org/a",
        "see https://example.org",
    ] {
        let evidence = Evidence::new(request, false);
        assert!(evidence.mentions_url(), "{request}");
        for name in ["web_fetch", "web_search"] {
            assert!(
                advertised(&evidence, &ResolvedTools::default(), name),
                "{name} follows {request}"
            );
        }
    }
    for request in ["fetch docs.example.org/a", "read example.org", ""] {
        let evidence = Evidence::new(request, false);
        assert!(!evidence.mentions_url(), "{request}");
        for name in ["web_fetch", "web_search"] {
            assert!(
                !advertised(&evidence, &ResolvedTools::default(), name),
                "{name} stays hidden for {request}"
            );
        }
    }
}

#[test]
fn a_configured_web_backend_advertises_the_web_pair_without_a_url() {
    let evidence = Evidence::new("inspect the repository", true);
    for name in ["web_fetch", "web_search"] {
        assert!(
            advertised(&evidence, &ResolvedTools::default(), name),
            "{name} follows the configuration"
        );
    }
}

#[test]
fn a_request_that_asks_for_helpers_advertises_them() {
    for request in [
        "use two helpers",
        "use one helper",
        "delegate this to a subagent",
        "run a sub-agent",
        "do these in parallel",
        "fan out the checks",
        "fan-out the checks",
        "delegate the search",
    ] {
        let evidence = Evidence::new(request, false);
        assert!(evidence.asks_for_helpers(), "{request}");
        assert!(
            advertised(&evidence, &ResolvedTools::default(), "helpers"),
            "helpers follow {request}"
        );
    }
    for request in ["inspect the repository", "parallelism is not requested"] {
        let evidence = Evidence::new(request, false);
        assert!(!evidence.asks_for_helpers(), "{request}");
        assert!(
            !advertised(&evidence, &ResolvedTools::default(), "helpers"),
            "helpers stay hidden for {request}"
        );
    }
}

#[test]
fn an_explicit_allow_entry_advertises_a_suppressed_tool() {
    for name in ["list", "delete", "web_fetch", "web_search"] {
        let tools = stated(&[name]);
        assert!(
            advertised(&Evidence::new("", false), &tools, name),
            "naming {name} advertises it"
        );
    }
    // `helpers` alone is refused as an empty helper set; a list that names it
    // beside the tools a helper may use is the valid shape.
    let tools = stated(&["helpers", "read"]);
    assert!(advertised(&Evidence::new("", false), &tools, "helpers"));
}

#[test]
fn an_explicit_allow_entry_does_not_advertise_a_tool_it_does_not_name() {
    let tools = stated(&["read"]);
    for name in ["list", "delete", "web_fetch", "web_search", "helpers"] {
        assert!(
            !advertised(&Evidence::new("", false), &tools, name),
            "{name} stays hidden"
        );
    }
}
