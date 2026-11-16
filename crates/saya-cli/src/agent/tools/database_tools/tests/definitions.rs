/// The retired `render_chart` model tool is advertised on no composition of
/// the database surface — this is the `saya ask` shape, which composes
/// `DatabaseTools` directly. The widest form (query data open, workspace
/// writes permitted) is checked alongside the narrowest.
#[test]
fn the_retired_render_chart_is_never_advertised() {
    for (allow_query_data, permit_workspace_writes) in [(false, false), (true, true)] {
        let tools =
            DatabaseTools::definitions(allow_query_data, true, true, permit_workspace_writes);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert!(
            !names.contains(&"render_chart"),
            "render_chart is retired; the database surface still advertises it: {names:?}"
        );
    }
}
