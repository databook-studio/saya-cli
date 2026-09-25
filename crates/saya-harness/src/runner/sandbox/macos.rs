//! The macOS Seatbelt profile generator: [`RunSandbox`](super::RunSandbox)
//! rendered into the draft profile (`docs/sandbox-profile-macos.sb.draft`),
//! whose every allow carries its measured reason (docs/sandbox-spike.md).
//!
//! Measured facts the generator must honour, all from the spike:
//! - `;` is the ONLY comment form `sandbox-exec` accepts (`/* */`, `//`, `#`
//!   are parse errors, exit 65) — every comment emitted here is `;`.
//! - `(allow file-read-data (literal "/"))` is required: without it every
//!   child dies SIGABRT before dyld prints anything (measured 10/10).
//! - Seatbelt matches the canonicalised path: every root substituted here was
//!   canonicalised at policy construction.
//! - The host position of a `(remote tcp ...)` rule accepts ONLY `*` or
//!   `localhost`, so `net_allow`'s host component is NOT enforced by this
//!   profile — selective egress to a named remote host is not expressible in
//!   this profile language on macOS 26.6.2. The host allowlist stays with the
//!   M3-1 fetch policy, enforced in-process; each rule says so.
//! - A quoted placeholder is silently accepted and matches nothing (measured:
//!   exit 0, fail-open on the allow side), so the generator itself refuses to
//!   emit any leftover `{...}` — the guard is here, not in `sandbox-exec`.

use std::{fmt::Write as _, path::PathBuf};

use super::{RunSandbox, SandboxError, validate};

/// The measured sandbox-exec path on this host (spike §1: present and
/// measuring on macOS 26.6.2; the startup probe re-checks presence).
pub(crate) const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// The literal text of the draft profile with every placeholder substituted.
/// The generated profile carries the same justifications, condensed to what
/// the child operator needs to read at the scene: what is allowed, why it was
/// measured, and what the profile deliberately does not cover.
///
/// The generator is public so the profile a reviewer (and the tests) see is
/// the one this module produces. The profile text alone is not a capability:
/// spawning a child goes through [`RunnerSpawn`](super::RunnerSpawn), handed
/// out only where the startup probe proved the sandbox.
pub fn seatbelt_profile(sb: &RunSandbox, program_dirs: &[PathBuf]) -> Result<String, SandboxError> {
    let mut p = String::new();
    p.push_str("; saya runner child sandbox — generated from docs/\n");
    p.push_str("; sandbox-profile-macos.sb.draft by the M5-3 generator.\n");
    p.push_str("; Deny by default; every allow below is measured or required.\n");
    p.push_str("; NOT covered here: the host component of net_allow (Seatbelt\n");
    p.push_str("; accepts only * or localhost in the remote filter; the host\n");
    p.push_str("; allowlist is enforced in-process by the M3-1 fetch policy),\n");
    p.push_str("; process-fork (denied; add only with a measured reason),\n");
    p.push_str("; mach-lookup, network-inbound, and all filesystem access\n");
    p.push_str("; outside the roots below.\n");
    p.push_str("(version 1)\n");
    p.push_str("(deny default)\n");
    if program_dirs.is_empty() {
        return Err(SandboxError::NoProgramDirs);
    }
    for dir in program_dirs {
        let _ = writeln!(p, "(allow process-exec (subpath \"{}\"))", dir.display());
    }
    // `(allow process-fork)` is granted only by explicit opt-in with a
    // measured reason (`RunSandbox::with_process_fork`): deny by default
    // measured a forking child as `fork: Operation not permitted` (spike
    // §4.5), and the reason the policy carries is emitted verbatim as the
    // comment a reviewer reads at the scene.
    if let Some(reason) = sb.process_fork_reason() {
        let _ = writeln!(p, "(allow process-fork) ; granted: {reason}");
    }
    // Loader startup, measured (spike §4.1): without this exact allow every
    // child dies SIGABRT before dyld prints anything — 10/10 for echo, mkdir,
    // cat, and bash. The mechanism is unverified; the requirement is measured.
    p.push_str("(allow file-read-data (literal \"/\"))\n");
    // Runtime init, measured (M5-4 escape battery): a Rust child queries a
    // sysctl at startup (`sysconf(_SC_PAGESIZE)` for its guard page) and
    // dies SIGABRT with `failed to allocate a guard page: Invalid argument`
    // when the query is denied — C binaries never touch it, so the spike's
    // canaries could not have caught it. The error class is EINVAL, not
    // EPERM, so the spike's deny-evidence rule does not apply here: the
    // allow is required by measurement, not by a deny log.
    p.push_str("(allow sysctl-read)\n");
    for root in sb.fs_roots() {
        let _ = writeln!(p, "(allow file-read* (subpath \"{}\"))", root.display());
        let _ = writeln!(p, "(allow file-write* (subpath \"{}\"))", root.display());
    }
    for (host, port) in sb.net_allow() {
        let remote =
            validate::seatbelt_host(host).ok_or_else(|| SandboxError::NetHostNotExpressible {
                host: host.clone(),
                platform: "macos",
                reason: "Seatbelt accepts only * or localhost in the remote filter",
            })?;
        // The host allowlist itself is enforced in-process (M3-1 fetch
        // policy): this rule names only the port-exact loopback/wildcard form
        // Seatbelt can express, and says so — it never silently narrows the
        // policy's host to something the profile could not honour.
        let _ = writeln!(
            p,
            "(allow network-outbound (remote tcp \"{remote}:{port}\")) ; host enforced \
             in-process only (M3-1 fetch policy)"
        );
    }
    // A leftover placeholder would silently match nothing (measured: a quoted
    // unknown subpath parses fine and fails open on the allow side) — refuse
    // to emit rather than hand out a permissive-looking half-profile.
    if p.contains('{') {
        return Err(SandboxError::PlaceholderLeft);
    }
    Ok(p)
}

/// Rejects a profile that still carries a placeholder — the same scan the
/// generator runs before emitting. `true` means the text is NOT safe to hand
/// to `sandbox-exec`: a quoted leftover is silently accepted and matches
/// nothing (measured), so the guard lives here, not in `sandbox-exec`.
pub fn has_leftover_placeholder(profile: &str) -> bool {
    profile.contains('{')
}
