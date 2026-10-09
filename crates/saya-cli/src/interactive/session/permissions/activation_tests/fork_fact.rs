use super::super::SESSION_FORK_FACT;

/// The fork fact says only what the running platform enforces (U8): the
/// macOS Seatbelt profile denies fork by omission — a child dies with
/// `fork: Operation not permitted` (`sandbox/macos.rs`, measured) — so
/// the clause says refused; the Linux confinement bounds reads, writes,
/// exec, and egress and nothing in it restricts fork
/// (`sandbox/mod.rs`: "a deliberate no-op"), so the clause says children
/// run — never "refused", the stronger false thing the clause once stated
/// on every platform.
#[test]
fn the_fork_fact_says_only_what_the_running_platform_enforces() {
    #[cfg(target_os = "macos")]
    {
        assert!(
            SESSION_FORK_FACT.contains("refused by the sandbox"),
            "macOS denies fork by profile omission — the clause states the refusal"
        );
        assert!(
            !SESSION_FORK_FACT.contains("restricts process-fork"),
            "macOS's clause is the denial, not an absence-of-restriction claim"
        );
    }
    #[cfg(target_os = "linux")]
    {
        assert!(
            SESSION_FORK_FACT.contains("restricts process-fork"),
            "Linux's confinement restricts no fork — the clause must say so"
        );
        assert!(
            SESSION_FORK_FACT.contains("children an interpreter spawns run"),
            "Linux children run; the clause states it"
        );
        assert!(
            !SESSION_FORK_FACT.contains("refused"),
            "Linux's clause must not claim the macOS denial — that is the \
             overstatement this fix exists to remove"
        );
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        assert!(
            SESSION_FORK_FACT.contains("no sandbox on this platform is proven"),
            "a platform without a proven sandbox path claims no fork fact"
        );
    }
}
