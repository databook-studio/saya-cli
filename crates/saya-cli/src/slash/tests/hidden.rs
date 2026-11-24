// K8: the `hidden` flag on a catalogue row. A hidden command still resolves,
// is still a command, and still parses — no behaviour is removed — but it is
// absent from the `/help` listing, the popup's completion, the palette and the
// typo suggestion. Three rows carry it: `/run` and `/runs` (O5) and
// `/recovery` (O6).

/// The hidden rows are `/run`, `/runs` (O5: out of scope for the new TUI,
/// which prints the `saya run` pointer) and `/recovery` (O6: hidden this
/// release, removed next). Hidden is presentation only: each still resolves,
/// is still a command, and still parses — no behaviour is removed.
#[test]
fn hidden_commands_resolve_and_parse_but_are_listed_nowhere() {
    let hidden: Vec<&str> = catalog::all()
        .filter(|spec| spec.hidden)
        .map(|spec| spec.name)
        .collect();
    assert_eq!(
        hidden,
        vec!["run", "runs", "recovery"],
        "the hidden rows, and only these"
    );

    let offered: Vec<&str> = catalog::entries().map(|entry| entry.name).collect();
    for name in &hidden {
        assert_eq!(
            catalog::resolve(name),
            Some(*name),
            "the hidden /{name} still resolves"
        );
        assert!(
            catalog::is_command(name),
            "the hidden /{name} is still a command"
        );
        assert!(
            parse_slash_command(&sample_line(name)).is_ok(),
            "the hidden /{name} still parses"
        );
        assert!(
            !offered.contains(name),
            "the hidden /{name} is offered nowhere"
        );
    }
}

/// The `/help` listing prints no line and no heading for a hidden command:
/// `/run` and `/runs` are the whole `Runs` section, so the empty heading goes
/// with them, and `/recovery` leaves the `Session` section.
#[test]
fn the_help_listing_prints_no_hidden_row() {
    let printed: Vec<(&str, &str)> = catalog::Category::ALL
        .into_iter()
        .flat_map(|category| {
            catalog::listed_in(category).map(move |(usage, _)| (category.heading(), usage))
        })
        .collect();
    assert_eq!(printed.len(), 39, "39 rows print: {} of them", printed.len());

    for name in ["run", "runs", "recovery"] {
        let own = format!("/{name} ");
        assert!(
            printed.iter().all(|(_, usage)| !usage.starts_with(&own)),
            "the listing prints /{name}: {printed:?}"
        );
    }
    let listing = help_text();
    assert!(
        !listing.lines().any(|line| line == "Runs"),
        "a heading with no row under it is not a listing: {listing}"
    );
}

/// The typo suggester never names a hidden command: a near-miss on `/run`,
/// `/runs` or `/recovery` suggests nothing rather than a command the listing
/// and the popup do not offer.
#[test]
fn closest_command_never_suggests_a_hidden_name() {
    let is_hidden = |name: &str| {
        catalog::all()
            .find(|spec| spec.name == name)
            .is_some_and(|spec| spec.hidden)
    };
    for input in ["run", "runs", "runns", "recovery", "recover", "recoverry"] {
        let suggestion = registry::closest_command(input);
        assert!(
            suggestion.is_none_or(|name| !is_hidden(name)),
            "the suggestion for {input:?} is a hidden command: {suggestion:?}"
        );
    }
    assert_eq!(
        registry::closest_command("recovery"),
        None,
        "an exact hidden spelling suggests nothing"
    );
    assert_eq!(
        registry::closest_command("runs"),
        None,
        "an exact hidden spelling suggests nothing"
    );
}
