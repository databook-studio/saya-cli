// One popup row: a name, its description, and where it exists.

/// A single completion-popup row.
pub(crate) struct Entry {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) surfaces: Surfaces,
}

/// Every command row, in parse order.
pub(crate) fn all() -> impl Iterator<Item = &'static CommandSpec> {
    CATALOGUE.iter()
}

/// Every name the parser accepts — canonical names then their aliases — in
/// catalogue order. The two production lookups want different cuts of it:
/// the popup offers [`entries`] and the suggester offers
/// [`suggestible_names`], so the full set is the catalogue tests' own view —
/// the parser's 63 names, hidden rows included.
#[allow(dead_code)]
pub(crate) fn names() -> impl Iterator<Item = &'static str> {
    all().flat_map(CommandSpec::names)
}

/// The popup's rows, in catalogue order: each canonical name then its aliases.
/// A hidden row is offered by nothing, so its names and aliases are absent.
pub(crate) fn entries() -> impl Iterator<Item = Entry> {
    all()
        .filter(|spec| !spec.hidden)
        .flat_map(|spec| {
            std::iter::once(Entry {
                name: spec.name,
                description: spec.description,
                surfaces: spec.surfaces,
            })
            .chain(spec.aliases.iter().map(|alias| Entry {
                name: alias.name,
                description: alias.description,
                surfaces: spec.surfaces,
            }))
        })
}

/// The canonical command `name` resolves to, alias spellings included.
pub(crate) fn resolve(name: &str) -> Option<&'static str> {
    all()
        .find(|spec| spec.names().any(|candidate| candidate == name))
        .map(|spec| spec.name)
}

/// Whether the parser accepts `name`, alias spellings included.
pub(crate) fn is_command(name: &str) -> bool {
    resolve(name).is_some()
}

/// The names the "did you mean" suggester may offer: every spelling of every
/// command that is not hidden. A hidden command still parses — it is simply
/// never suggested.
pub(crate) fn suggestible_names() -> impl Iterator<Item = &'static str> {
    all()
        .filter(|spec| !spec.hidden)
        .flat_map(CommandSpec::names)
}

/// The `/help` listing of one category: `(usage, description)` in listing
/// order, ranked by [`CommandSpec::listed`]. A hidden row keeps its listing
/// data and prints nothing.
pub(crate) fn listed_in(category: Category) -> impl Iterator<Item = (&'static str, &'static str)> {
    let mut rows: Vec<(u8, &'static str, &'static str)> = all()
        .filter(|spec| !spec.hidden)
        .filter(|spec| spec.category == category)
        .filter_map(|spec| {
            spec.listed
                .map(|(usage, rank)| (rank, usage, spec.description))
        })
        .collect();
    rows.sort_by_key(|(rank, _, _)| *rank);
    rows.into_iter()
        .map(|(_, usage, description)| (usage, description))
}
