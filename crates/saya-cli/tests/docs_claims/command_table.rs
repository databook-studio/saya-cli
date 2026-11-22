//! `docs/commands.md` states the slash command table, and the table is the
//! catalogue's: a command the page names must be one the surface offers, in
//! the same words `/help` prints, with the same aliases.
//!
//! The page cannot read `CATALOGUE`, so the block between the two markers is
//! generated here from the catalogue's own rows. A page that stopped matching
//! fails with the whole expected block, which is the one thing to paste: add a
//! row in `crates/saya-cli/src/slash/catalog.rs`, run this suite, copy the
//! expected block into the page.
//!
//! A hidden command is left out: it resolves and runs when typed, and the page
//! offers none of it.

use saya_cli::documented_commands;

/// The markers the page wraps the generated block in.
const BEGIN: &str = "<!-- generated: slash commands -->";
const END: &str = "<!-- end: slash commands -->";

/// The table, rendered from the catalogue.
fn render() -> String {
    let mut out = String::from("| Command | Arguments | What it does | Aliases |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for row in documented_commands() {
        let arguments = if row.args.is_empty() {
            "—".to_owned()
        } else {
            cell(&format!("`{}`", row.args))
        };
        let aliases = if row.aliases.is_empty() {
            "—".to_owned()
        } else {
            row.aliases
                .iter()
                .map(|alias| format!("`/{alias}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "| `/{}` | {arguments} | {} | {aliases} |\n",
            row.name,
            cell(row.description)
        ));
    }
    out.trim_end().to_owned()
}

/// One cell, with the pipe a usage line may state escaped.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// The generated block the page holds, or `None` when it holds none.
fn block(page: &str) -> Option<&str> {
    let start = page.find(BEGIN)? + BEGIN.len();
    let end = page[start..].find(END)? + start;
    Some(page[start..end].trim())
}

#[test]
fn the_slash_command_table_is_generated_from_the_catalogue() {
    let page = include_str!("../../../../docs/commands.md");
    let expected = render();
    let Some(found) = block(page) else {
        panic!(
            "docs/commands.md has no generated slash command block: put `{BEGIN}` and `{END}` \
             around the table. The block must read:\n\n{expected}\n"
        );
    };
    assert_eq!(
        found, expected,
        "docs/commands.md's slash command table drifted from \
         crates/saya-cli/src/slash/catalog.rs: replace the block between `{BEGIN}` and `{END}` \
         with the table above"
    );
}

#[test]
fn the_table_offers_every_listed_command_and_no_hidden_one() {
    let rows = documented_commands();
    assert!(
        rows.len() >= 30,
        "the docs table lists every offered command; only {} rows",
        rows.len()
    );
    for hidden in ["run", "runs", "recovery"] {
        assert!(
            !rows.iter().any(|row| row.name == hidden),
            "`/{hidden}` is hidden from every surface, so the page offers it nowhere"
        );
    }
    let page = include_str!("../../../../docs/commands.md");
    let found = block(page).unwrap_or_default();
    for row in &rows {
        assert!(
            found.contains(&format!("| `/{}` |", row.name)),
            "the page must list `/{name}`: {found}",
            name = row.name
        );
    }
}
