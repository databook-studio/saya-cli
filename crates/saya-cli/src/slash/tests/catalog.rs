// K1: the catalogue is the one list of slash commands. These tests pin the
// two directions (parser → catalogue, catalogue → parser), the alias
// resolution, the typo suggester's alias coverage, and the absence of a
// second hand-maintained list anywhere in `saya-cli`'s sources.

/// The commands the parser's `match name` answers to, read from its source.
///
/// The parser stays a `match`, so the only way to prove the catalogue covers
/// it is to read the arms: every top-level arm pattern is indented eight
/// spaces by `rustfmt`, which is what separates them from the inner matches
/// inside arm bodies (`/sessions`, `/tasks`). The retired `/coding` arm is a
/// message, not a command, so it is not a name the catalogue owes a row.
fn parser_arm_names() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/slash/dispatch.rs");
    let source = std::fs::read_to_string(&path).expect("the parser source is readable");
    let mut names = Vec::new();
    let mut arm = String::new();
    for line in source.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        if arm.is_empty() && (indent != 8 || !trimmed.starts_with('"')) {
            continue;
        }
        arm.push_str(trimmed);
        arm.push(' ');
        let Some((pattern, _)) = arm.split_once("=>") else {
            continue;
        };
        for part in pattern.split('|') {
            let literal = part.trim().trim_matches('"');
            if !literal.is_empty() && literal != "coding" && !literal.contains(' ') {
                names.push(literal.to_string());
            }
        }
        arm.clear();
    }
    names
}

/// The catalogue's one-line description for a name, alias spellings included.
fn description_of(name: &str) -> Option<&'static str> {
    catalog::entries()
        .find(|entry| entry.name == name)
        .map(|entry| entry.description)
}

/// The line that shows one command parsing: bare `/name` where the grammar
/// allows it, plus the argument the grammar requires where it does not.
fn sample_line(name: &str) -> String {
    let tail = match name {
        "start" => " survey the data",
        "follow-up" => " j-1 a --expected-completion-sequence 3 go",
        "job" => " list",
        "approve" | "deny" => " j-1",
        "message" => " j-1 a hi",
        "pause" | "continue" | "cancel-child" => " j-1 a",
        "connect" | "include" | "exclude" => " prod",
        "resume" => " s-1",
        "investigation" => " list",
        "sql" => " select 1",
        "export" => " out.csv",
        "report" => " out.md",
        "run" => " survey --allow workspace-write",
        "allow" => " sql:analytics",
        "confirm" | "reject" => " ab12",
        "remember" => " analytics.public.orders description one row per order",
        "forget" => " c-1",
        _ => "",
    };
    format!("/{name}{tail}")
}

/// Both directions, counted: the parser answers 59 command names, the
/// catalogue lists the same 59, and every catalogued name parses.
#[test]
fn catalogue_and_parser_name_the_same_commands() {
    let arms = parser_arm_names();
    assert_eq!(arms.len(), 59, "the parser's match has 59 command arms");
    let catalogued: Vec<&str> = catalog::names().collect();
    assert_eq!(catalogued.len(), 59, "the catalogue has 59 names");

    let missing: Vec<&String> = arms
        .iter()
        .filter(|name| !catalogued.contains(&name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "these parser arms have no catalogue row: {missing:?}"
    );
    let orphaned: Vec<&&str> = catalogued
        .iter()
        .filter(|name| !arms.iter().any(|arm| arm == *name))
        .collect();
    assert!(
        orphaned.is_empty(),
        "these catalogue names have no parser arm: {orphaned:?}"
    );

    for name in &catalogued {
        let line = sample_line(name);
        assert!(
            parse_slash_command(&line).is_ok(),
            "catalogue name /{name} does not parse: {}",
            parse_slash_command(&line).unwrap_err().0
        );
    }
}

/// Every alias resolves to the command it spells, carries its own description
/// (so the popup row reads as today), and parses.
#[test]
fn every_alias_resolves_to_its_command() {
    let mut aliases: Vec<(&str, &str)> = Vec::new();
    for spec in catalog::all() {
        for alias in spec.aliases {
            aliases.push((alias.name, spec.name));
            assert_eq!(
                catalog::resolve(alias.name),
                Some(spec.name),
                "{} must resolve to /{}",
                alias.name,
                spec.name
            );
            assert_eq!(
                description_of(alias.name),
                Some(alias.description),
                "{} keeps its own one-line description",
                alias.name
            );
            assert!(
                parse_slash_command(&format!("/{}", alias.name)).is_ok(),
                "alias /{} does not parse",
                alias.name
            );
        }
    }
    assert_eq!(
        aliases,
        vec![
            ("approvals", "mode"),
            ("investigations", "investigation"),
            ("contract", "contracts"),
            ("quit", "exit"),
        ],
        "today's aliases, and only these"
    );
}

/// The typo suggester answers for alias spellings too, so a near-miss on an
/// alias lands on the alias rather than on nothing.
#[test]
fn closest_command_suggests_for_aliases() {
    assert_eq!(registry::closest_command("quitt"), Some("quit"));
    assert_eq!(registry::closest_command("investigationz"), Some("investigation"));
    assert_eq!(registry::closest_command("connectt"), Some("connect"));
}

/// No second hand-maintained list survives: the four old tables are gone from
/// `saya-cli`'s sources. The needle is assembled from parts so this test does
/// not match its own text.
#[test]
fn no_second_command_list_exists() {
    let needles = [
        concat!("KNOWN_", "COMMANDS"),
        concat!("COMMAND_", "DESCRIPTIONS"),
        concat!("LISTING_", "GROUPS"),
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir).expect("a readable source directory");
        for entry in entries {
            let path = entry.expect("a readable source entry").path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("a readable source file");
            for (number, line) in source.lines().enumerate() {
                for needle in needles {
                    if line.contains(needle) {
                        offenders.push(format!("{}:{}: {needle}", path.display(), number + 1));
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a second command list is back:\n{}",
        offenders.join("\n")
    );
}

/// `/setup` is the one command that needs the full-screen terminal today: the
/// catalogue says so, and the popup (a TUI surface) keeps it.
#[test]
fn setup_is_the_one_full_screen_only_command() {
    for spec in catalog::all() {
        let expected = spec.name != "setup";
        assert_eq!(
            spec.surfaces.covers(catalog::Surface::Line),
            expected,
            "/{} is full-screen only: the catalogue must say so, and say it once",
            spec.name
        );
        assert!(spec.surfaces.covers(catalog::Surface::Tui));
    }
}

/// The `/help` listing is a view of the catalogue: every canonical row is
/// listed, carries a description and its own usage line, ranks are unique,
/// and the folded aliases are the only names the listing does not show.
#[test]
fn the_help_listing_is_one_ranked_view_of_the_catalogue() {
    let mut ranks: Vec<u8> = Vec::new();
    for spec in catalog::all() {
        let (usage, rank) = spec
            .listed
            .unwrap_or_else(|| panic!("/{} is not listed today", spec.name));
        assert!(
            usage.starts_with(&format!("/{}", spec.name)),
            "/{} is listed with someone else's usage: {usage}",
            spec.name
        );
        assert!(
            !spec.description.is_empty(),
            "/{} is listed without a description",
            spec.name
        );
        ranks.push(rank);
    }
    assert_eq!(ranks.len(), 55, "the listing shows 55 of today's 59 names");
    assert_eq!(
        catalog::names().count(),
        59,
        "with the four folded aliases, the parser answers 59 names"
    );
    ranks.sort_unstable();
    let mut unique = ranks.clone();
    unique.dedup();
    assert_eq!(ranks, unique, "two listing rows share a rank");
}
