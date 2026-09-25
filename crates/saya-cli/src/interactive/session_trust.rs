//! The startup trust prompt (G3, Decision 2 §1-4) and the bypass × trust
//! intersection (Decision 2, "Both decisions together").
//!
//! At startup, before the first turn, a fresh session that bound no root
//! asks once on a terminal — trust this folder for the session, name a
//! different directory instead, or continue unbound. Trust is session-only:
//! nothing is remembered, so the trusted folder binds exactly like an
//! explicit `--workspace` and pins into the session record; a resume
//! re-opens the pin and never re-prompts. The prompt must not become
//! inference with a confirmation step: trust binds exactly the launch cwd
//! or the typed directory — never a parent, never a walk. Where no terminal
//! exists (piped stdin, `--turn-file`), nothing binds and nothing prompts:
//! today's unbound shape plus G1's notice.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// The startup trust prompt's bytes: the fact, the consequence, the exits —
/// and the session-only shape. Asked once, at startup, on a terminal.
pub(crate) const TRUST_PROMPT: &str = "No workspace is bound: this folder is not inside a git \
    worktree and no `--workspace` was given, so file tools and `run_program` are unavailable \
    and the host lane cannot compose. Trust this folder as the session workspace? [t]rust once \
    / [w]orkspace <dir> instead / [c]ontinue unbound. (Startup only; nothing is remembered.)";

/// The bypass × unbound × non-terminal note: bypass runs with the lane
/// absent — the fail-closed intersection — and says so.
pub(crate) const BYPASS_UNBOUND_NO_LANE: &str = "bypass on with no workspace bound: \
    run_command is unavailable — the folder prompt needs a terminal, so nothing bound; \
    launch with `--workspace <dir>` or answer the trust prompt on a terminal.";

const MAX_TRUST_READS: usize = 8;

/// One answer to the trust prompt: trust the launch cwd, bind the typed
/// directory instead, or continue unbound — today's shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrustAnswer {
    TrustCwd,
    Workspace(PathBuf),
    ContinueUnbound,
}

/// The prompt's inputs, read once at startup: terminal presence, session
/// freshness, the launch's own statements, and whether anything bound.
/// `root_bound` distinguishes "a root is bound, stay silent" from "nothing
/// bound, ask"; `turn_file` marks the single-turn headless path even where
/// a caller passes a terminal bit through.
#[derive(Debug, Clone)]
pub(crate) struct TrustPromptContext {
    pub(crate) is_terminal: bool,
    pub(crate) fresh: bool,
    pub(crate) has_explicit: bool,
    pub(crate) has_pin: bool,
    pub(crate) root_bound: bool,
    pub(crate) turn_file: bool,
}

/// Whether the startup trust prompt appears: a fresh session that bound no
/// root, on a terminal, with no single-turn file to run. Resumes never
/// prompt — a pinned resume re-opens its pin, an unbound `--continue` keeps
/// today's shape — and an explicit `--workspace` (or an already-bound root)
/// already stated the answer.
pub(crate) fn should_prompt(ctx: &TrustPromptContext) -> bool {
    ctx.is_terminal
        && ctx.fresh
        && !ctx.turn_file
        && !ctx.root_bound
        && !ctx.has_pin
        && !ctx.has_explicit
}

/// Parses one trust answer line: `t`/`trust` trusts the cwd, `w <dir>`
/// binds the typed directory exactly, `c`/`continue` keeps today's unbound
/// shape. Anything else is an error the caller re-asks on — never a bind.
pub(crate) fn parse_trust_answer(line: &str) -> Result<TrustAnswer, String> {
    let trimmed = line.trim();
    if trimmed.eq_ignore_ascii_case("t") || trimmed.eq_ignore_ascii_case("trust") {
        return Ok(TrustAnswer::TrustCwd);
    }
    if trimmed.eq_ignore_ascii_case("c")
        || trimmed.eq_ignore_ascii_case("continue")
        || trimmed.eq_ignore_ascii_case("continue unbound")
    {
        return Ok(TrustAnswer::ContinueUnbound);
    }
    let Some(tail) = trimmed
        .strip_prefix('w')
        .or_else(|| trimmed.strip_prefix('W'))
    else {
        return Err(format!(
            "unrecognized answer {trimmed:?}: answer `t` (trust once), `w <dir>` \
             (a different directory), or `c` (continue unbound)"
        ));
    };
    let dir = tail.trim();
    if dir.is_empty() {
        return Err("`w` names a directory: `w <dir>` binds that directory instead".to_owned());
    }
    Ok(TrustAnswer::Workspace(PathBuf::from(dir)))
}

/// Resolves the trusted directory: canonicalised, must exist, must be a
/// directory — exactly the named dir, never a parent, never a walk. The same
/// rule an explicit `--workspace` follows, without the flag's wording.
pub(crate) fn resolve_trusted_dir(dir: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(dir).map_err(|error| {
        format!(
            "the trusted folder {} could not be resolved: {error}; name an existing directory",
            dir.display()
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "the trusted folder {} is not a directory; name an existing directory",
            dir.display()
        ));
    }
    Ok(canonical)
}

/// Asks the trust prompt once at startup: prints the prompt, reads answers
/// until one parses. An unparseable line re-asks; EOF (a vanishing stdin)
/// continues unbound rather than hanging or refusing startup. The `w <dir>`
/// answer's directory is resolved here, so the caller binds exactly what was
/// typed — no parent, no walk.
pub(crate) fn ask_trust(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<TrustAnswer, String> {
    for _ in 0..MAX_TRUST_READS {
        writeln!(output, "{TRUST_PROMPT}")
            .map_err(|error| format!("the trust prompt could not be written: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("the trust prompt could not be written: {error}"))?;
        let mut line = String::new();
        let read = input
            .read_line(&mut line)
            .map_err(|error| format!("the trust answer could not be read: {error}"))?;
        if read == 0 {
            return Ok(TrustAnswer::ContinueUnbound);
        }
        match parse_trust_answer(&line) {
            Ok(TrustAnswer::Workspace(dir)) => {
                let resolved = resolve_trusted_dir(&dir)?;
                return Ok(TrustAnswer::Workspace(resolved));
            }
            Ok(answer) => return Ok(answer),
            Err(_) => continue,
        }
    }
    Ok(TrustAnswer::ContinueUnbound)
}

/// The trust answer's echo: names the just-trusted tree and the session-only
/// shape — the half of the moment-of-choice pair the lane fact does not
/// carry. Said where the trust answer lands, beside the bypass line under
/// bypass.
pub(crate) fn trusted_root_line(root: &Path) -> String {
    format!(
        "workspace trusted for this session only: {} (nothing is remembered)",
        root.display()
    )
}

/// The bypass × unbound × non-terminal note: `Some` exactly when bypass runs
/// with the lane absent — the fail-closed intersection — so bypass says the
/// lane is gone. `None` wherever the lane composed or the mode is not
/// bypass: nothing exceptional to say.
pub(crate) fn bypass_no_lane_note(bypass: bool, host_composed: bool) -> Option<String> {
    (bypass && !host_composed).then(|| BYPASS_UNBOUND_NO_LANE.to_owned())
}
