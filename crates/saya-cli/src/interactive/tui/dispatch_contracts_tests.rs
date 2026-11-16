use super::*;
use crate::cli::ForgetReasonArg;

// The active profile a /connect set, differing from any configured default.
const ACTIVE: &str = "staging";

/// Every `ContractsCommand` variant that carries a `profile` field must take
/// the session's active profile when routed through the TUI — the same field
/// the headless `--profile` flag sets. A variant that falls through loses the
/// profile and silently reads the configured default, defeating cross-profile
/// isolation. `/queue` is where a human confirms a claim, so the queue showing
/// another profile's candidates is the worst case.
#[test]
fn with_profile_stamps_active_profile_on_every_profiled_contract_variant() {
    // `/queue` parses to Queue { profile: None, limit }; the TUI must route it
    // at the active profile, not the configured default.
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: Some(20),
    };
    assert_eq!(
        with_profile(&queue, Some(ACTIVE)),
        ContractsCommand::Queue {
            profile: Some(ACTIVE.into()),
            limit: Some(20),
        },
        "/queue lost the active profile — it would read the configured default's candidates"
    );

    // The other profile-bearing read/write variants, for completeness.
    assert_eq!(
        with_profile(&ContractsCommand::List { profile: None }, Some(ACTIVE)),
        ContractsCommand::List {
            profile: Some(ACTIVE.into()),
        }
    );
    assert_eq!(
        with_profile(
            &ContractsCommand::Show {
                table: "a.b.c".into(),
                profile: None,
            },
            Some(ACTIVE),
        ),
        ContractsCommand::Show {
            table: "a.b.c".into(),
            profile: Some(ACTIVE.into()),
        }
    );
    assert_eq!(
        with_profile(
            &ContractsCommand::Remember {
                table: "a.b.c".into(),
                kind: crate::cli::ClaimKindArg::Alias,
                value: "v".into(),
                column: None,
                reason: None,
                profile: None,
            },
            Some(ACTIVE),
        ),
        ContractsCommand::Remember {
            table: "a.b.c".into(),
            kind: crate::cli::ClaimKindArg::Alias,
            value: "v".into(),
            column: None,
            reason: None,
            profile: Some(ACTIVE.into()),
        }
    );
}

/// The claim-keyed variant (`Forget`) has no profile field — it addresses a
/// claim by id, so the active profile must NOT be injected. This guards
/// against an over-broad fix that stamps a profile where none exists. (`Decide`
/// is claim-keyed too but carries a `profile` field it stamps, above.)
#[test]
fn with_profile_leaves_claim_keyed_variants_untouched() {
    let forget = ContractsCommand::Forget {
        claim_id: "c-1".into(),
        reason: ForgetReasonArg::UserRequest,
    };
    assert_eq!(with_profile(&forget, Some(ACTIVE)), forget);
}

/// `None` active profile (no /connect yet) must not invent one: the command
/// keeps `profile: None` and the dispatcher resolves the configured default.
#[test]
fn with_profile_none_keeps_none() {
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    assert_eq!(
        with_profile(&queue, None),
        ContractsCommand::Queue {
            profile: None,
            limit: None,
        }
    );
}
