//! The programs no allowlist may ever name: shells and interpreters.
//!
//! A typed-argv runner and its program allowlist do not exist; what remains is
//! the refusal list the config layer consults so a shell or interpreter can
//! never be declared where a program name is expected.

/// The programs a program allowlist may never name, whatever it says: shells
/// and interpreters. A program allowlist names one program run with typed
/// argv — an interpreter can spawn arbitrary children with arbitrary argv, so
/// allowing one would void that contract from inside the allowlist. Shared by
/// config resolution, which refuses the name at resolve time.
pub fn is_refused_runner_program(name: &str) -> bool {
    const REFUSED: &[&str] = &[
        "sh",
        "bash",
        "dash",
        "zsh",
        "ksh",
        "csh",
        "tcsh",
        "fish",
        "env",
        "perl",
        "python",
        "python3",
        "ruby",
        "node",
        "php",
        "lua",
        "awk",
        "osascript",
        "expect",
        "tclsh",
        "swift",
        "script",
    ];
    REFUSED.contains(&name)
}
