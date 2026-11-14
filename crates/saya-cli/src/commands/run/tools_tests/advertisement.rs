// A run step's goal is its request text: the lean default set applies to the
// tools a run advertises, and `[tools] allow` still narrows or opts back in.

fn built(goal: &str, tools: &saya_config::ResolvedTools, label: &str) -> Vec<String> {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let ws = workspace(label);
    let steps = [step(goal, Capabilities::default())];
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: None,
            workspace: &ws,
            allow_query_data: true,
            tools,
            mcp: None,
            hooks: None,
        },
        &steps,
    );
    names(&built, 0)
}

#[test]
fn a_run_step_advertises_the_lean_default_set() {
    let advertised = built(
        "inspect the repository",
        &saya_config::ResolvedTools::default(),
        "lean-run-default",
    );
    for name in ["read", "glob", "grep"] {
        assert!(
            advertised.contains(&name.to_owned()),
            "{name}: {advertised:?}"
        );
    }
    assert!(!advertised.contains(&"list".to_owned()), "{advertised:?}");
}

#[test]
fn an_explicit_allow_entry_readvertises_list_for_a_run_step() {
    let tools = saya_config::ResolvedTools::from_stated(Some(&["list".to_owned()]))
        .expect("list is a governed tool");
    let advertised = built("inspect the repository", &tools, "lean-run-allow-list");
    assert!(advertised.contains(&"list".to_owned()), "{advertised:?}");
}
