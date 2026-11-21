// K3: the knowledge store is one command, `/contracts <verb>`. These tests pin
// the catalogue's side of that: one listed row carrying the eight verbs, the
// five old names folded in as aliases (never listed as commands), and every
// verb in the listed usage actually parsing.

/// The one-line description the popup and the listing show for `name`.
fn popup_description(name: &str) -> Option<&'static str> {
    catalog::entries()
        .find(|entry| entry.name == name)
        .map(|entry| entry.description)
}

/// The listed usage line of one canonical command.
fn listed_usage(name: &str) -> &'static str {
    catalog::all()
        .find(|spec| spec.name == name)
        .and_then(|spec| spec.listed)
        .map(|(usage, _rank)| usage)
        .unwrap_or_else(|| panic!("/{name} is not listed"))
}

/// `/help` lists `/contracts` with its verbs, and no longer lists the five old
/// spellings as commands of their own.
#[test]
fn the_listing_shows_one_contracts_row_with_its_verbs() {
    let listing = help_text();
    assert!(
        listing.contains("/contracts <list|show|queue|confirm|reject|confirm-all|export|import>"),
        "the listing shows the /contracts verb row: {listing}"
    );
    for old in ["/queue", "/confirm", "/reject", "/approve-all"] {
        assert!(
            !listing.contains(old),
            "{old} is an alias; the listing shows canonical rows only: {listing}"
        );
    }
    // `/contract` is a prefix of `/contracts`, so assert on whole listing rows.
    for line in listing.lines() {
        let trimmed = line.trim_start();
        assert!(
            !trimmed.starts_with("/contract "),
            "/contract is an alias and must not be listed: {listing}"
        );
    }
}

/// The five old spellings are catalogue aliases of `/contracts`, each saying so
/// in its own popup row; none of them is a canonical row any more.
#[test]
fn the_five_old_names_are_aliases_of_contracts() {
    for old in ["contract", "queue", "confirm", "reject", "approve-all"] {
        assert_eq!(
            catalog::resolve(old),
            Some("contracts"),
            "/{old} resolves to /contracts"
        );
        let description = popup_description(old).unwrap_or_else(|| panic!("/{old} has a popup row"));
        assert!(
            description.to_lowercase().contains("deprecated alias"),
            "the popup row for /{old} says it is a deprecated alias: {description}"
        );
        assert!(
            description.contains("/contracts"),
            "the popup row for /{old} names the canonical spelling: {description}"
        );
        assert!(
            !catalog::all().any(|spec| spec.name == old),
            "/{old} is no longer a canonical row"
        );
    }
}

/// The listed usage line is the grammar: every verb word in it parses, so a
/// reader can copy a verb and run it.
#[test]
fn every_verb_in_the_listed_usage_parses() {
    let usage = listed_usage("contracts");
    let verbs = usage
        .split_once('<')
        .and_then(|(_, rest)| rest.split_once('>'))
        .map(|(verbs, _)| verbs)
        .expect("the /contracts usage carries its verb list in brackets");
    let verbs: Vec<&str> = verbs.split('|').collect();
    assert_eq!(
        verbs,
        vec![
            "list",
            "show",
            "queue",
            "confirm",
            "reject",
            "confirm-all",
            "export",
            "import"
        ],
        "the eight verbs, in the order the row states them"
    );
    for verb in verbs {
        // Every verb takes an argument except `list` and `queue`.
        let tail = match verb {
            "show" => " analytics.public.orders",
            "confirm" | "reject" => " ki-a86a3f",
            "export" | "import" => " out/context.json",
            _ => "",
        };
        let line = format!("/contracts {verb}{tail}");
        assert!(
            parse_slash_command(&line).is_ok(),
            "the listed verb does not parse: {line}"
        );
    }
    // The canonical row's description names the store, not one operation.
    let description = popup_description("contracts").expect("/contracts is described");
    assert!(
        description.contains("knowledge store"),
        "the /contracts row describes the store: {description}"
    );
}

/// `/remember` and `/forget` are kept as short verbs of their own: canonical
/// rows, listed, and documented — unchanged by the merge.
#[test]
fn remember_and_forget_stay_commands_of_their_own() {
    for name in ["remember", "forget"] {
        assert!(
            catalog::all().any(|spec| spec.name == name),
            "/{name} is still a canonical row"
        );
        assert_eq!(catalog::resolve(name), Some(name));
        assert!(
            !popup_description(name)
                .unwrap_or_default()
                .to_lowercase()
                .contains("deprecated"),
            "/{name} is not deprecated: {}",
            popup_description(name).unwrap_or_default()
        );
    }
}
