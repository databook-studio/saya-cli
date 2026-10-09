/// The `--allow` help text exactly as clap derives it from the doc
/// comment — the string a user reads in `saya run --help`.
fn clap_allow_help() -> String {
    use clap::CommandFactory as _;
    let mut cmd = crate::cli::Cli::command();
    let run = cmd
        .find_subcommand_mut("run")
        .expect("`saya run` is declared");
    let allow = run
        .get_arguments()
        .find(|arg| arg.get_id() == "allow")
        .expect("`--allow` is declared on `saya run`");
    allow
        .get_long_help()
        .or_else(|| allow.get_help())
        .expect("the --allow doc comment reaches clap")
        .to_string()
}

/// The grammar's families, derived from [`KNOWN`] itself: each token
/// paired with its family key (the prefix before `:`, the name
/// [`NOT_YET_WIRED`] entries use). A token added to [`KNOWN`] without a
/// surface update fails the parity test, not just the parse.
fn grammar_tokens() -> Vec<(&'static str, &'static str)> {
    let list = KNOWN
        .split_once("known scopes: ")
        .expect("`KNOWN` must name the scopes it refuses around")
        .1;
    list.split(", ")
        .map(|token| {
            let family = token.split(':').next().unwrap_or(token);
            (family, token)
        })
        .collect()
}

/// True when some clause of the surface pairs the scope token with a
/// refusal word — the shape a refusal claim takes in these surfaces. A
/// clause is a `.`-sentence cut further at `;`, `:` (outside the token's
/// own shape), and `(`/`)`, so a refusal scoped to one family inside a
/// parenthetical (the `/allow` help's `command:` lane gate) does not leak
/// onto every token the outer sentence lists ("the wired scopes are …").
/// A real disagreement still reads: a token sharing its own clause with a
/// refusal word stays red in both directions.
fn claims_refused(surface: &str, token: &str) -> bool {
    surface.split('.').any(|sentence| {
        sentence.contains(token)
            && sentence
                .split([';', '(', ')'])
                .any(|clause| clause.contains(token) && clause.contains("refus"))
    })
}

/// The reader above must stay able to tell a scoped refusal from list
/// prose: a refusal word that shares the token's own parenthetical is a
/// claim about that token, while one cut off by the same boundaries —
/// the `/allow` lane gate living one clause away from the wired list — is
/// not. A reader that could not make that distinction either cries wolf
/// on every listed scope or goes blind to a real disagreement; both
/// halves are pinned here so a future edit cannot weaken one into the
/// other.
#[test]
fn the_refusal_reader_attributes_a_refusal_to_its_own_clause() {
    let allow = crate::slash::command_help("allow").expect("/allow has per-command help");
    assert!(allow.contains("workspace-write"));
    assert!(
        !claims_refused(allow, "workspace-write"),
        "the /allow lane gate sits in command:'s own clause, not workspace-write's: {allow}"
    );
    assert!(
        claims_refused(allow, "endpoint:<role>=<endpoint>"),
        "endpoint:'s own clause does refuse it, and the reader must say so: {allow}"
    );
    let scoped = "the wired scopes are `workspace-write`, and `sql:<connection>`. \
            `endpoint:<role>=<endpoint>` is refused here: nothing binds it.";
    assert!(
        claims_refused(scoped, "endpoint:<role>=<endpoint>"),
        "a real refusal claim must still read as one: {scoped}"
    );
    assert!(
        !claims_refused(scoped, "workspace-write"),
        "a listed scope one sentence away must not read as refused: {scoped}"
    );
}
