//! The macOS Seatbelt profile generator: [`RunSandbox`](super::RunSandbox)
//! rendered into the draft profile (`docs/sandbox-profile-macos.sb.draft`),
//! whose every allow carries its measured reason (docs/sandbox-spike.md).
//!
//! Measured facts the generator must honour:
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
//!   fetch policy, enforced in-process; each rule says so.
//! - A quoted placeholder is silently accepted and matches nothing (measured:
//!   exit 0, fail-open on the allow side), so the generator itself refuses to
//!   emit any leftover `{...}` — the guard is here, not in `sandbox-exec`.
//! - A program directory is emitted with `file-read-metadata` beside its
//!   `process-exec`, and `/dev/null` is readable: both are measured
//!   requirements of a child that runs a **shell** (`docs/standards/…`), which
//!   searches `PATH` and reopens `/dev/null` for its background jobs. Neither
//!   widens a read/write root — the first is metadata on a directory already
//!   trusted for exec, the second is one device, not a tree.

use std::{fmt::Write as _, path::PathBuf};

use super::{RunSandbox, SandboxError, validate};

/// The measured sandbox-exec path on this host (present and measuring on
/// macOS 26.6.2; the startup probe re-checks presence).
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
        // A program directory is also searchable. Measured: with
        // `process-exec` alone, `/bin/sh` reports `touch: command not found`
        // for `/usr/bin/touch` even though that path is exec-allowed — a
        // shell resolves a bare name through `PATH` by stat'ing each
        // directory and each candidate, and a denied stat is indistinguishable
        // from a missing file. Metadata only: this never allows reading the
        // contents of any file in the directory.
        let _ = writeln!(
            p,
            "(allow file-read-metadata (subpath \"{}\"))",
            dir.display()
        );
    }
    // The shell's own background-job semantics, measured: POSIX assigns
    // `/dev/null` to an asynchronous list's standard input, so `sleep 600 &`
    // fails with `cannot redirect standard input from /dev/null: Operation
    // not permitted` unless the device can be reopened by the child. Read
    // only — the device is not a filesystem root and carries no data.
    p.push_str("(allow file-read* (literal \"/dev/null\"))\n");
    // `(allow process-fork)` is granted only by explicit opt-in with a
    // measured reason (`RunSandbox::with_process_fork`): deny by default
    // measured a forking child as `fork: Operation not permitted`, and the
    // reason the policy carries is emitted verbatim as the
    // comment a reviewer reads at the scene.
    if let Some(reason) = sb.process_fork_reason() {
        let _ = writeln!(p, "(allow process-fork) ; granted: {reason}");
    }
    // Loader startup, measured: without this exact allow every
    // child dies SIGABRT before dyld prints anything — 10/10 for echo, mkdir,
    // cat, and bash. The mechanism is unverified; the requirement is measured.
    p.push_str("(allow file-read-data (literal \"/\"))\n");
    // Runtime init, measured by the escape battery: a Rust child queries a
    // sysctl at startup (`sysconf(_SC_PAGESIZE)` for its guard page) and
    // dies SIGABRT with `failed to allocate a guard page: Invalid argument`
    // when the query is denied — C binaries never touch it, so the
    // canaries could not have caught it. The error class is EINVAL, not
    // EPERM, so the deny-evidence rule does not apply here: the
    // allow is required by measurement, not by a deny log.
    p.push_str("(allow sysctl-read)\n");
    for root in sb.fs_roots() {
        let _ = writeln!(p, "(allow file-read* (subpath \"{}\"))", root.display());
        let _ = writeln!(p, "(allow file-write* (subpath \"{}\"))", root.display());
    }
    // Toolchain directories: readable and executable, never writable. A
    // sandboxed `cargo test` needs the compiler, its libraries, and the
    // programs on the extended PATH; the checkout remains the only writable
    // tree. `process-exec` and metadata are emitted beside the read rule for
    // the same reason the program directories carry them: a shell resolves a
    // bare name by stat'ing it, and exec needs the directory's own allow.
    for root in sb.read_only_roots() {
        let _ = writeln!(p, "(allow process-exec (subpath \"{}\"))", root.display());
        let _ = writeln!(
            p,
            "(allow file-read-metadata (subpath \"{}\"))",
            root.display()
        );
        let _ = writeln!(p, "(allow file-read* (subpath \"{}\"))", root.display());
    }
    for (host, port) in sb.net_allow() {
        let remote =
            validate::seatbelt_host(host).ok_or_else(|| SandboxError::NetHostNotExpressible {
                host: host.clone(),
                platform: "macos",
                reason: "Seatbelt accepts only * or localhost in the remote filter",
            })?;
        // The host allowlist itself is enforced in-process (the fetch
        // policy): this rule names only the port-exact loopback/wildcard form
        // Seatbelt can express, and says so — it never silently narrows the
        // policy's host to something the profile could not honour.
        let _ = writeln!(
            p,
            "(allow network-outbound (remote tcp \"{remote}:{port}\")) ; host enforced \
             in-process only (M3-1 fetch policy)"
        );
    }
    // A write denial inside a write root, emitted LAST: Seatbelt is
    // last-match-wins, so a `(deny file-write* (subpath ...))` placed after
    // the root's `(allow file-write* ...)` denies exactly that subtree. The
    // path was resolved against a canonical root at construction, so it
    // carries the shape Seatbelt matches. This is the one place the profile
    // is deliberately narrower than its roots.
    for path in sb.write_denied() {
        let _ = writeln!(p, "(deny file-write* (subpath \"{}\"))", path.display());
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;
    use crate::runner::sandbox::RunSandbox;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// A canonical scratch root, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "saya-h2-profile-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("the scratch root must be creatable");
            Self(fs::canonicalize(&path).expect("the scratch root must canonicalise"))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A write denial inside a write root is emitted exactly once, and after
    /// every `allow file-write*` line — Seatbelt is last-match-wins, so the
    /// position is what makes the denial bite.
    #[test]
    fn a_write_denial_is_emitted_after_every_write_allow() {
        let root = Scratch::new();
        let denied = root.0.join(".git");
        let sb = RunSandbox::new([root.0.clone()], Vec::<(String, u16)>::new())
            .expect("a real root constructs")
            .with_write_denied(denied.clone())
            .expect("the denial is inside the root");
        let profile =
            seatbelt_profile(&sb, &[PathBuf::from("/bin")]).expect("the profile generates");
        let deny = format!("(deny file-write* (subpath \"{}\"))", denied.display());
        assert_eq!(
            profile.matches(&deny).count(),
            1,
            "exactly one deny line must be emitted:\n{profile}"
        );
        let deny_at = profile.find(&deny).expect("the deny line is present");
        let last_allow = profile
            .rfind("(allow file-write*")
            .expect("a write allow is present");
        assert!(
            deny_at > last_allow,
            "the deny must come after every write allow:\n{profile}"
        );
        assert!(!has_leftover_placeholder(&profile), "{profile}");
    }

    /// A read-only root is readable and executable, never writable: the
    /// toolchain reaches a sandboxed build without widening the checkout.
    #[test]
    fn a_read_only_root_is_never_emitted_as_a_write_allow() {
        let root = Scratch::new();
        let toolchain = Scratch::new();
        let sb = RunSandbox::new([root.0.clone()], Vec::<(String, u16)>::new())
            .expect("a real root constructs")
            .with_read_only_roots([toolchain.0.clone()])
            .expect("the toolchain root constructs");
        let profile =
            seatbelt_profile(&sb, &[PathBuf::from("/bin")]).expect("the profile generates");
        for rule in [
            format!("(allow file-read* (subpath \"{}\"))", toolchain.0.display()),
            format!(
                "(allow process-exec (subpath \"{}\"))",
                toolchain.0.display()
            ),
            format!(
                "(allow file-read-metadata (subpath \"{}\"))",
                toolchain.0.display()
            ),
        ] {
            assert!(profile.contains(&rule), "missing {rule}:\n{profile}");
        }
        assert!(
            !profile.contains(&format!(
                "(allow file-write* (subpath \"{}\"))",
                toolchain.0.display()
            )),
            "a read-only root must never be writable:\n{profile}"
        );
    }
}
