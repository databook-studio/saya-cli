//! `--allow <scopes>` parsing: the CLI side of the run approval.
//!
//! A headless run cannot prompt, so its capability set must be stated before
//! anything starts — and it must be stated exactly: an unknown scope name is
//! a usage error, never a silently ignored token (a run that believes it
//! approved more or less than the user typed is a lying run). The grammar
//! mirrors the [`Capabilities`] contract fields; the built approval is
//! exactly the stated scopes, nothing implicit.

use saya_types::{Capabilities, Destination, EndpointBindings, FetchScope, RunnerScope};

/// The scope grammar, for the error message that names what was refused.
const KNOWN: &str = "known scopes: none, workspace-write, scratch, \
                     fetch:<scheme>+<host>, runner:<program>, endpoint:<role>=<endpoint>";

/// Scopes the grammar accepts but the run engine cannot yet act on: no tool
/// in a run's universe consumes them, so approving one would gate nothing.
///
/// They are refused at parse time rather than accepted and ignored. The
/// repo's own standard is that a flag implying a capability is available is
/// worse than no flag — a user who types `--allow scratch` and gets a run
/// has been told the model may use a scratch database, and it cannot. An
/// independent review found exactly this shipped, and this list is the fix.
///
/// Each entry names the plan item that wires it. Deleting an entry is the
/// whole of "turning the scope on" once its tool is in the universe.
const NOT_YET_WIRED: &[(&str, &str)] = &[(
    "endpoint",
    "every episode calls the orchestrator endpoint; per-step roles are not bound yet",
)];

/// The refusal for a scope that parses but binds nothing.
fn not_yet_wired(token: &str, family: &str) -> Option<String> {
    NOT_YET_WIRED
        .iter()
        .find(|(name, _)| *name == family)
        .map(|(_, why)| {
            format!(
                "scope `{token}` is not available yet: {why}. It parses, but nothing in a run \
             would consume it, so approving it would gate nothing. Re-run without it."
            )
        })
}

/// The scopes `--allow` approved.
#[derive(Debug)]
pub(super) struct Approved {
    pub(super) capabilities: Capabilities,
}

/// Parses the `--allow` tokens. An empty list is the caller's refusal
/// decision, not a silently-empty approval; anything here that does not
/// match the grammar is a typed usage error. `none` is the grammar's
/// explicit empty approval — see the head of the body.
pub(super) fn parse(tokens: &[String]) -> Result<Approved, String> {
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
        });
    }
    let mut capabilities = Capabilities::default();
    let mut destinations = Vec::new();
    let mut programs = Vec::new();
    let mut bindings = Vec::new();
    for token in tokens {
        if token == "workspace-write" {
            capabilities.workspace_write = true;
        } else if token == "scratch" {
            capabilities.scratch = true;
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
        } else if let Some(rest) = token.strip_prefix("runner:") {
            if let Some(refusal) = not_yet_wired(token, "runner") {
                return Err(refusal);
            }
            programs.push(rest.to_string());
        } else if let Some(rest) = token.strip_prefix("endpoint:") {
            if let Some(refusal) = not_yet_wired(token, "endpoint") {
                return Err(refusal);
            }
            let Some((role, endpoint)) = rest.split_once('=') else {
                return Err(format!(
                    "scope `{token}` must be endpoint:<role>=<endpoint>; {KNOWN}"
                ));
            };
            bindings.push((role.to_string(), endpoint.to_string()));
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
    if !programs.is_empty() {
        capabilities.runner = Some(
            RunnerScope::new(programs).map_err(|error| format!("runner scope refused: {error}"))?,
        );
    }
    if !bindings.is_empty() {
        capabilities.endpoints = EndpointBindings::new(bindings)
            .map_err(|error| format!("endpoint bindings refused: {error}"))?;
    }
    Ok(Approved { capabilities })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An independent review found `--allow scratch`, `fetch:`, `runner:` and
    /// `endpoint:` accepted, persisted and rendered while no tool in a run's
    /// universe consumed any of them. Approving a capability that gates
    /// nothing tells the user something false about what the model may do,
    /// so each is refused until its wiring lands. `scratch` wired with its
    /// tool (S1), `fetch` (S2) and `runner` (S3) left this list; the rest
    /// stay refused, and this test keeps refusing them the day they are
    /// typed.
    #[test]
    fn a_scope_nothing_consumes_is_refused_rather_than_silently_approved() {
        let token = "endpoint:analyst=fast";
        let Err(error) = parse(&[token.to_string()]) else {
            panic!("`{token}` gates nothing and must be refused");
        };
        assert!(
            error.contains("not available yet"),
            "the refusal must say why, got: {error}"
        );
        assert!(
            error.contains(token),
            "the refusal must name the scope, got: {error}"
        );
    }

    /// The reason every entry here is refused is wiring, not absence: each
    /// named capability exists in the run engine, and a run's tool universe
    /// simply does not consume it yet. An absence claim goes stale the
    /// moment the tool lands — the exact lie `runner:` shipped after M5-4's
    /// `run_program` merged. The list is the single source of the
    /// user-facing reason text, so each entry is pinned to the one phrasing
    /// that is true — a future drift from it is a diff in this test rather
    /// than a lie to the user. (`endpoint` names no tool: its true reason
    /// is that per-step roles are not bound, so it is pinned to its own.)
    #[test]
    fn a_refusal_names_the_wiring_reason_never_an_absence_claim() {
        for (family, why) in NOT_YET_WIRED {
            assert!(
                !why.contains("not exist"),
                "`{family}`'s refusal claims a tool is absent — the refusal \
                 class is wiring, not absence: {why}"
            );
            assert!(
                why.contains("per-step roles are not bound"),
                "`{family}`'s refusal must state its true reason verbatim, \
                 got: {why}"
            );
        }
    }

    /// The scope that *is* wired keeps working — the fix must refuse the
    /// inert ones without breaking the capability a run can actually use.
    #[test]
    fn workspace_write_is_wired_and_still_approves() {
        let Ok(approved) = parse(&["workspace-write".to_string()]) else {
            panic!("the wired scope must still approve");
        };
        assert!(approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.runner.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// The inverse pin's first half: `scratch` is wired, so `--allow scratch`
    /// approves the scope — the deletion of its refusal entry alone proves
    /// nothing, this does. The second half (its tool in the universe of the
    /// steps that asked for it) lives beside the toolset builder's tests.
    #[test]
    fn scratch_is_wired_and_still_approves() {
        let Ok(approved) = parse(&["scratch".to_string()]) else {
            panic!("the wired scope must approve");
        };
        assert!(approved.capabilities.scratch);
        assert!(!approved.capabilities.workspace_write);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.runner.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// The inverse pin's first half: `runner:` is wired, so `--allow
    /// runner:bench` approves the scope — the deletion of its refusal entry
    /// alone proves nothing, this does. The second half (its tool in the
    /// universe of the steps that asked, and the narrowed-allowlist gate
    /// that rides the toolset builder) lives beside the toolset builder's
    /// tests.
    #[test]
    fn runner_is_wired_and_still_approves() {
        let Ok(approved) = parse(&["runner:bench".to_string()]) else {
            panic!("the wired scope must approve");
        };
        let runner = approved
            .capabilities
            .runner
            .expect("the runner scope must be approved");
        assert_eq!(runner.programs, vec!["bench".to_owned()]);
        assert!(!approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// The empty approval is stateable: `--allow none` starts a run that
    /// approves nothing at all. One of the grammar's five capability
    /// scopes is refused, so without this token the only way to start any
    /// run — including a purely read-only one — would be approving one of
    /// the wired scopes, which would turn a scope that means something into
    /// boilerplate everyone types.
    #[test]
    fn none_states_the_empty_approval_for_a_read_only_run() {
        let Ok(approved) = parse(&["none".to_string()]) else {
            panic!("`none` must state the empty approval");
        };
        assert!(!approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.runner.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// `none` names "nothing", so beside a scope it would state both
    /// nothing and something — refused, not resolved in the user's favour.
    #[test]
    fn none_must_be_the_only_token_when_stated() {
        let Err(error) = parse(&["none".to_string(), "workspace-write".to_string()]) else {
            panic!("`none` beside a scope must refuse");
        };
        assert!(
            error.contains("`none` states the empty scope set"),
            "the refusal must name the contradiction: {error}"
        );
        assert!(error.contains("known scopes:"), "got: {error}");
    }

    /// An unknown token stays a usage error, and its message still lists the
    /// grammar — the refusal for "not yet" must not swallow the refusal for
    /// "no such thing".
    #[test]
    fn an_unknown_scope_is_still_a_usage_error_naming_the_grammar() {
        let Err(error) = parse(&["wat".to_string()]) else {
            panic!("unknown scope must refuse");
        };
        assert!(error.contains("unknown scope `wat`"), "got: {error}");
        assert!(error.contains("known scopes:"), "got: {error}");
    }
}
