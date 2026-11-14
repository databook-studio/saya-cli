/// The scopes the approval carries: the stated tokens, with the fetch
/// family normalised to the URL parser's spelling at parse — the run
/// engine's fetch policy compares the lowercased destination, and the grant
/// token suggester produces the same form from the call's URL, so a seeded
/// token and a suggested token are one string for one destination. A
/// session's `/allow` seeds these words, so `/grants` shows the grammar's
/// form of what was granted. A run's approval is `capabilities` plus the
/// frozen decider's seeds. `command:` tokens build no run capability and
/// ride the tokens verbatim (the run surface never produces them — it
/// refuses them above — so they only flow on the session surface).
#[derive(Debug)]
pub(crate) struct Approved {
    pub(crate) capabilities: Capabilities,
    pub(crate) tokens: Vec<String>,
}

/// The replacement rule the removed runner/interpreter doors are refused
/// with on the run surface: one wording for both families, stating what a
/// run's steps do instead. Pinned verbatim by the tests, so it cannot drift
/// into a vaguer claim. The session surface's own wording is the session
/// module's (`REMOVED_RUNNER_GRANT_REFUSAL`): a run's replacement is the
/// plan's `shell` tool, a session's is its shell tool.
pub(crate) const REMOVED_DOOR_REFUSAL: &str =
    "runner and interpreter scopes were removed; plan steps run commands with the shell tool";

/// The stated tokens that are per-call grant words — the families that bind
/// no run capability (`Capabilities` has no field for them), so they ride
/// the approval's tokens verbatim: the frozen decider seeds them and the
/// journal's `PlanApproved` payload carries them, in stated order, deduped.
/// Today that family is `sql:` only.
pub(crate) fn carried_tokens(tokens: &[String]) -> Vec<String> {
    let mut carried = Vec::new();
    for token in tokens {
        if token.starts_with("sql:") && !carried.contains(token) {
            carried.push(token.clone());
        }
    }
    carried
}

/// Parses the `--allow` tokens. An empty list is the caller's refusal
/// decision, not a silently-empty approval; anything here that does not
/// match the grammar is a typed usage error. `none` is the grammar's
/// explicit empty approval — see the head of the body.
///
/// `pub(crate)` so the interactive session's grant token suggester and the
/// `/allow` command can feed every token they produce or accept back
/// through this one parser — the grammar's authority is here, never a
/// duplicate. The `surface` selects which [`NOT_YET_WIRED`] refusals apply;
/// the grammar itself is one loop, one `KNOWN`, one shape rule set.
pub(crate) fn parse(tokens: &[String], surface: Surface) -> Result<Approved, String> {
    // `none` states the empty approval: no capabilities at all, read-only
    // by construction — the episode's per-tool-call decider already
    // defaults to read-only (`assembly.rs`), and nothing a refused scope
    // would gate is reachable. It must stand alone: beside a scope it
    // would state both "nothing" and "something", which states nothing.
    if tokens.iter().any(|token| token == "none") {
        if tokens.len() > 1 {
            return Err(format!(
                "`none` states the empty scope set and must be the only token; {KNOWN}"
            ));
        }
        return Ok(Approved {
            capabilities: Capabilities::default(),
            tokens: tokens.to_vec(),
        });
    }
    // The grammar's words as the approval carries them, normalised where
    // the engine normalises: a fetch destination's spelling is the URL
    // parser's — lowercased scheme and host — and the run engine's fetch
    // policy compares exactly that form, as does the grant token suggester
    // spelling a call's URL. Normalising at parse, on both surfaces, makes
    // a seeded token and a suggested token one string for one destination:
    // whatever casing the user typed, the grant pre-answers the call it
    // names. Every other family rides the stated spelling
    // verbatim: connection names are case-sensitive identifiers nothing in
    // the engine folds, so normalising them would widen a grant across
    // distinct names.
    let stated: Vec<String> = tokens
        .iter()
        .map(|token| match token.split_once(':') {
            Some(("fetch", rest)) => format!("fetch:{}", rest.to_ascii_lowercase()),
            _ => token.clone(),
        })
        .collect();
    let mut capabilities = Capabilities::default();
    let mut destinations = Vec::new();
    let mut bindings = Vec::new();
    for token in &stated {
        if token == "workspace-write" {
            capabilities.workspace_write = true;
        } else if let Some(rest) = token.strip_prefix("fetch:") {
            let Some((scheme, host)) = rest.split_once('+') else {
                return Err(format!(
                    "scope `{token}` must be fetch:<scheme>+<host>; {KNOWN}"
                ));
            };
            let destination = Destination::new(scheme, host).map_err(|_| {
                format!("scope `{token}` is not a scheme plus a bare host; {KNOWN}")
            })?;
            destinations.push(destination);
        } else if token.starts_with("runner:") || token.starts_with("interpreter:") {
            // The runner and interpreter scopes are refused: a run's steps
            // run commands with the `shell` tool, so no plan scope names a
            // program list. The words still *parse* — a persisted plan or
            // spec naming them is not an unknown scope — but each surface
            // refuses them with its own replacement rule: the run's wording
            // names the plan, the session's names the grant families it does
            // not carry. No surface builds a runner or interpreter
            // capability from them.
            if surface == Surface::Run {
                return Err(format!("scope `{token}`: {REMOVED_DOOR_REFUSAL}"));
            }
            return Err(format!(
                "scope `{token}`: {REMOVED_RUNNER_GRANT_REFUSAL}. Re-issue /allow without it."
            ));
        } else if let Some(rest) = token.strip_prefix("command:") {
            // The host lane's own family: payload a bare name, shells
            // included — the `runner:`/`interpreter:` family mirror
            // deliberately does not apply here (`command:bash` is the token,
            // and the warning rides the prompt). On the run surface this is
            // a permanent policy refusal in its own class, never an absence
            // claim; on the session surface it parses and the composition
            // gate (`allow_refusal`) decides whether this session carries it.
            if !is_bare_name(rest) {
                return Err(format!(
                    "scope `{token}` must be command:<program>, a bare name — never a path \
                     or traversal; {KNOWN}"
                ));
            }
            if surface == Surface::Run {
                return Err(run_command_refusal(token));
            }
        } else if let Some(rest) = token.strip_prefix("endpoint:") {
            // The payload is judged by the grammar's own shape rule first:
            // a refusal message that says "it parses" must be true.
            let Some((role, endpoint)) = rest.split_once('=') else {
                return Err(format!(
                    "scope `{token}` must be endpoint:<role>=<endpoint>; {KNOWN}"
                ));
            };
            if !is_name_shaped(role) || !is_name_shaped(endpoint) {
                return Err(format!(
                    "scope `{token}` must name a role and an endpoint with the shape a \
                     run-scoped name has; {KNOWN}"
                ));
            }
            if let Some(refusal) = not_yet_wired(token, "endpoint", surface) {
                return Err(refusal);
            }
            bindings.push((role.to_string(), endpoint.to_string()));
        } else if let Some(rest) = token.strip_prefix("sql:") {
            // The payload is a connection's registry name, judged by the
            // same name-shape rule `endpoint:` payloads are. The token builds
            // no run capability: it is a per-call grant word, carried on
            // `Approved::tokens` — the run's frozen decider seeds it and the
            // journal records it.
            if !is_name_shaped(rest) {
                return Err(format!(
                    "scope `{token}` must be sql:<connection>, a connection's registry \
                     name; {KNOWN}"
                ));
            }
        } else {
            return Err(format!("unknown scope `{token}`; {KNOWN}"));
        }
    }
    if !destinations.is_empty() {
        capabilities.fetch = Some(
            FetchScope::new(destinations)
                .map_err(|error| format!("fetch scope refused: {error}"))?,
        );
    }
    if !bindings.is_empty() {
        capabilities.endpoints = EndpointBindings::new(bindings)
            .map_err(|error| format!("endpoint bindings refused: {error}"))?;
    }
    Ok(Approved {
        capabilities,
        tokens: stated,
    })
}
