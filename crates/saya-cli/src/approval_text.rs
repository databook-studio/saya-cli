//! The interpreter approval's warning sentence — the one builder both
//! surfaces that grant an interpreter share, so they cannot drift: the run's
//! plan-approval view (`commands/run/approval_view.rs`), and the session's
//! bypass activation line (`interactive::session_activation`). No euphemism,
//! no dilution into a generic "dangerous mode" banner: the sentence states
//! what is being accepted in place of the typed-argv contract's behavioural
//! half.

/// The warning sentence for `programs`, named by the surface's subject
/// ("run", "session") and that surface's process-fork clause. The run may
/// grant process-fork, so its clause is conditional; the session's clause
/// is the running platform's own fact — on macOS a forked child dies with
/// `fork: Operation not permitted` (`sandbox/mod.rs`, measured), on Linux
/// nothing in the confinement restricts fork — so the clause is the
/// caller's, never a shared half-truth.
pub(crate) fn interpreter_warning(subject: &str, programs: &[String], fork_fact: &str) -> String {
    format!(
        "interpreter approval: this {subject} may execute {} as an interpreter. Its argv is \
         typed and the sandbox still bounds its reads, writes, exec, and egress — but the \
         model writes the program the interpreter runs, and {fork_fact} What the interpreter \
         computes is not a reviewed, fixed binary.",
        programs.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The run surface's bytes are today's bytes, exactly: the conditional
    /// clause, the subject, and the closing sentence all in place. The
    /// builder is shared; the run's wording must not move under it.
    #[test]
    fn the_run_surface_s_warning_keeps_its_bytes() {
        let warning = interpreter_warning(
            "run",
            &["python3".to_string()],
            "(where process-fork is granted) any children it spawns run arbitrary argv.",
        );
        assert_eq!(
            warning,
            "interpreter approval: this run may execute python3 as an interpreter. Its argv \
             is typed and the sandbox still bounds its reads, writes, exec, and egress — but \
             the model writes the program the interpreter runs, and (where process-fork is \
             granted) any children it spawns run arbitrary argv. What the interpreter \
             computes is not a reviewed, fixed binary."
        );
    }

    /// The clause rides the builder verbatim — the sample here is the
    /// macOS clause's bytes; the platform's own clause is pinned in
    /// `session_activation_tests`
    /// (`the_fork_fact_says_only_what_the_running_platform_enforces`).
    /// The run's parenthetical — "(where process-fork is granted)" — is a
    /// run's clause and must not ride along with a session subject.
    #[test]
    fn the_session_s_clause_names_the_fork_fact() {
        let warning = interpreter_warning(
            "session",
            &["python3".to_string()],
            "no process-fork is granted: children an interpreter spawns are refused by \
             the sandbox.",
        );
        assert!(warning.contains("this session may execute python3"));
        assert!(warning.contains(
            "no process-fork is granted: children an interpreter spawns are refused \
                 by the sandbox"
        ));
        assert!(!warning.contains("where process-fork is granted"));
    }
}
