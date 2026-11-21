// `/undo` and `/rewind`: the two grammars, and the catalogue rows that list
// them. `/undo` takes no argument; `/rewind` takes none (a listing) or exactly
// one checkpoint id, carried as stated and resolved against the run's listing.

/// `/undo` is bare: a trailing word is a mistake worth naming, never a silent
/// ignore.
#[test]
fn undo_takes_no_argument() {
    assert_eq!(parse_slash_command("/undo"), Ok(Some(SlashCommand::Undo)));
    let error = parse_slash_command("/undo 2").unwrap_err().0;
    assert!(
        error.contains("undo takes no argument"),
        "the refusal names the grammar: {error}"
    );
}

/// `/rewind` bare lists; one word names the checkpoint restored; two words are
/// a usage error. The id is carried as stated — the operation resolves it
/// against the listing, so a typo is refused by name rather than read as
/// another turn.
#[test]
fn rewind_parses_a_listing_and_one_checkpoint_id() {
    assert_eq!(
        parse_slash_command("/rewind"),
        Ok(Some(SlashCommand::Rewind(RewindRequest::List)))
    );
    assert_eq!(
        parse_slash_command("/rewind 3"),
        Ok(Some(SlashCommand::Rewind(RewindRequest::Restore(
            "3".to_owned()
        ))))
    );
    assert_eq!(
        parse_slash_command("/rewind   ck-12  "),
        Ok(Some(SlashCommand::Rewind(RewindRequest::Restore(
            "ck-12".to_owned()
        )))),
        "the id is trimmed and carried verbatim"
    );
    let error = parse_slash_command("/rewind 1 2").unwrap_err().0;
    assert!(
        error.contains("rewind takes one checkpoint id"),
        "two ids are refused, not resolved by picking one: {error}"
    );
}

/// Both commands are in the one catalogue, listed with their own usage, under
/// the session heading — so `/help`, the popup and the typo suggester all know
/// them from the one table.
#[test]
fn the_catalogue_lists_undo_and_rewind() {
    for (name, usage) in [("undo", "/undo"), ("rewind", "/rewind [<id>]")] {
        let spec = catalog::all()
            .find(|spec| spec.name == name)
            .unwrap_or_else(|| panic!("/{name} has no catalogue row"));
        assert_eq!(
            spec.listed.map(|(usage, _)| usage),
            Some(usage),
            "/{name} is listed with its own usage"
        );
        assert!(
            !spec.description.is_empty(),
            "/{name} is listed without a description"
        );
        assert!(spec.surfaces.covers(catalog::Surface::Line));
        assert!(spec.surfaces.covers(catalog::Surface::Tui));
        assert_eq!(catalog::resolve(name), Some(name));
    }
}
