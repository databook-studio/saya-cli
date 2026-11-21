use super::*;

/// The eight verbs the merged command names, each parsed to the operation the
/// headless `saya contracts` subcommand runs.
#[test]
fn the_eight_verbs_are_the_knowledge_store() {
    assert_eq!(
        command_of("/contracts list"),
        ContractsCommand::List { profile: None }
    );
    assert_eq!(
        command_of("/contracts show analytics.public.orders"),
        ContractsCommand::Show {
            table: "analytics.public.orders".into(),
            profile: None,
        }
    );
    assert_eq!(
        command_of("/contracts queue 20"),
        ContractsCommand::Queue {
            profile: None,
            limit: Some(20),
        }
    );
    for (verb, decision) in [
        ("confirm", ReviewDecisionArg::Confirm),
        ("reject", ReviewDecisionArg::Reject),
    ] {
        assert_eq!(
            command_of(&format!("/contracts {verb} ki-a86a3f")),
            ContractsCommand::Decide {
                prefix: "ki-a86a3f".into(),
                decision,
                profile: None,
            }
        );
    }
    assert_eq!(
        command_of("/contracts confirm-all --yes 5"),
        ContractsCommand::ApproveAll {
            profile: None,
            limit: Some(5),
            yes: true,
        }
    );
    assert_eq!(
        command_of("/contracts export out/context.json"),
        ContractsCommand::Export {
            path: "out/context.json".into(),
            profile: None,
            overwrite: false,
        }
    );
    assert_eq!(
        command_of("/contracts import in/context.json --preview"),
        ContractsCommand::Import {
            path: "in/context.json".into(),
            profile: None,
            preview: true,
        }
    );
}

/// Each deprecated spelling is the same verb under the other name: for every
/// accepted argument shape the two spellings parse to the same operation, so a
/// reader moving from the old name to the new one changes nothing but the
/// words.
#[test]
fn each_deprecated_spelling_is_the_same_verb() {
    for (old, new) in [
        ("/queue", "/contracts queue"),
        ("/queue 20", "/contracts queue 20"),
        ("/confirm ki-a86a3f", "/contracts confirm ki-a86a3f"),
        ("/reject ki-a86a3f", "/contracts reject ki-a86a3f"),
        ("/approve-all", "/contracts confirm-all"),
        ("/approve-all --yes 10", "/contracts confirm-all --yes 10"),
    ] {
        assert_eq!(command_of(old), command_of(new), "{old} vs {new}");
    }
    // `/contract` is the deprecated spelling of `/contracts` itself.
    for (old, new) in [
        ("/contract", "/contracts"),
        ("/contracts list", "/contracts"),
        (
            "/contract analytics.public.orders",
            "/contracts show analytics.public.orders",
        ),
    ] {
        assert_eq!(command_of(old), command_of(new), "{old} vs {new}");
    }
}

/// Today's one-token form still works: a lone token that is not a verb word is
/// the table to show. The verb words win over a table of the same name — a
/// documented cost of one command.
#[test]
fn a_lone_non_verb_token_is_still_a_show() {
    assert_eq!(
        command_of("/contracts analytics.public.orders"),
        ContractsCommand::Show {
            table: "analytics.public.orders".into(),
            profile: None,
        }
    );
    // `list` is a verb now, so it lists rather than showing a table named
    // "list".
    assert_eq!(
        command_of("/contracts list"),
        ContractsCommand::List { profile: None }
    );
}

/// The verb grammar refuses extra tokens with a payload-free message: the
/// untrusted tail never reaches the usage line. Only whole words count as an
/// echo — a one-letter token can occur inside the usage's own placeholder — and
/// the grammar's own words are not payload.
#[test]
fn the_verb_grammar_refuses_extra_tokens_payload_free() {
    const GRAMMAR: [&str; 10] = [
        "list",
        "show",
        "queue",
        "confirm",
        "reject",
        "confirm-all",
        "export",
        "import",
        "import-dbt",
        "--yes",
    ];
    for (name, arg) in [
        ("contracts", "show a b"),
        ("contracts", "list extra"),
        ("contracts", "queue lots"),
        ("contracts", "confirm"),
        ("contracts", "confirm a b"),
        ("contracts", "confirm-all kaboom"),
        ("contracts", "a b"),
    ] {
        let bad = parse_contract_command(name, arg).unwrap_err();
        assert!(!bad.0.is_empty(), "/{name} {arg} must say how to spell it");
        let words: Vec<&str> = bad
            .0
            .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
            .filter(|word| !word.is_empty())
            .collect();
        for token in arg.split_whitespace() {
            // The verb words and the one flag are the grammar, not payload: the
            // usage line is allowed to name them.
            if GRAMMAR.contains(&token) {
                continue;
            }
            assert!(
                !words.contains(&token),
                "/{name} {arg} echoed {token:?} in its usage: {}",
                bad.0
            );
        }
    }
}
