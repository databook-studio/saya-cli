// The documented view of [`CATALOGUE`]: the rows the release documentation's
// table lists, in the order `/help` reads them.
//
// The page is prose and cannot be checked by the compiler, so
// `tests/docs_claims/command_table.rs` renders it from this projection and
// fails when `docs/commands.md` states another table. Nothing here is a
// second catalogue: a row is a borrow of the catalogue's own fields, and a
// hidden command has no row because the page offers none of it.

/// One catalogue row as the documentation's table lists it.
pub struct DocumentedCommand {
    /// The canonical spelling, without the `/`.
    pub name: &'static str,
    /// The arguments `/help` prints after the command, or empty for one that
    /// takes none. The alias note an alias-bearing row carries in its usage
    /// line is left out: the alias column states it.
    pub args: &'static str,
    /// The one-line description: the `/help` listing line and the popup row.
    pub description: &'static str,
    /// The other spellings that resolve to this command.
    pub aliases: Vec<&'static str>,
}

/// Every command the documentation lists, in catalogue order. A hidden row is
/// left out: it resolves and runs when typed, and is offered nowhere.
pub fn documented_commands() -> Vec<DocumentedCommand> {
    CATALOGUE
        .iter()
        .filter(|spec| !spec.hidden)
        .map(|spec| DocumentedCommand {
            name: spec.name,
            args: arguments(spec),
            description: spec.description,
            aliases: spec.aliases.iter().map(|alias| alias.name).collect(),
        })
        .collect()
}

/// The usage line's arguments: the text after the command's first word, with
/// the `(alias /quit)`-shaped note some rows carry removed.
fn arguments(spec: &CommandSpec) -> &'static str {
    let Some((usage, _)) = spec.listed else {
        return "";
    };
    let rest = usage
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest.trim());
    match rest.strip_prefix("(alias ") {
        Some(_) => "",
        None => rest,
    }
}
