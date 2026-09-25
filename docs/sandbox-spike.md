# Sandbox spike (M5-3): what the OS sandboxes actually do — measured

- **Status:** spike evidence and draft decision, in the shape ADR 0003
  established (measured facts → decision → sign-off). Not an ADR; the M5-3
  implementation slice amends/creates the ADR from this note.
- **Date:** 2026-09-11
- **Deliverables:** this note, the probe
  ([`crates/saya-harness/tests/sandbox_probe.rs`](../crates/saya-harness/tests/sandbox_probe.rs)),
  and two profile drafts:
  [`sandbox-profile-macos.sb.draft`](sandbox-profile-macos.sb.draft),
  [`sandbox-profile-linux.md.draft`](sandbox-profile-linux.md.draft).
- **What was measured:** macOS only. **Linux is UNMEASURED** — this host is
  macOS, and nothing in this document claims a Linux behaviour that was
  observed. Windows is UNMEASURED and, by design, has nothing to measure (U9).
- **No production module, no `landlock` dependency, no `#[allow(...)]`** — this
  slice adds one test file and three documents.

## 1. Host and tooling

All macOS measurements were taken on the machine running this spike, unprivileged
(no root), `rustc` 1.98.0:

```console
$ sw_vers
ProductName:		macOS
ProductVersion:		26.6.2
BuildVersion:		25G83
$ uname -a
Darwin Subodhs-MacBook-Pro.local 25.6.0 Darwin Kernel Version 25.6.0: Fri Jul 31 19:16:36 PDT 2026; root:xnu-12377.161.14~5/RELEASE_ARM64_T6030 arm64
$ ls -la /usr/bin/sandbox-exec
-rwxr-xr-x  1 root  wheel  102368 Aug 13 12:51 /usr/bin/sandbox-exec
```

The probe is `crates/saya-harness/tests/sandbox_probe.rs`; run it with:

```console
$ cargo test -p saya-harness --test sandbox_probe -- --nocapture
```

It writes a full report to `$CARGO_TARGET_TMPDIR/sandbox-probe-report.txt` and
echoes it. The verdict is fail closed by construction
(`ProbeReport::proves_runner`): the runner may be registered on a host only if
every **required** check passed — an optional check failing is recorded context,
and a platform the probe cannot measure records that explicitly (Windows,
unsupported unix) rather than being absent.

## 2. Verbatim probe report (final run on this host)

```text
platform: macos
kernel: Darwin Subodhs-MacBook-Pro.local 25.6.0 Darwin Kernel Version 25.6.0: Fri Jul 31 19:16:36 PDT 2026; root:xnu-12377.161.14~5/RELEASE_ARM64_T6030 arm64
verdict: runner PROVEN on this host

[PASS] optional check host_uname
    Darwin Subodhs-MacBook-Pro.local 25.6.0 Darwin Kernel Version 25.6.0: Fri Jul 31 19:16:36 PDT 2026; root:xnu-12377.161.14~5/RELEASE_ARM64_T6030 arm64

[PASS] required check sandbox_exec_present
    /usr/bin/sandbox-exec exists on this host

[PASS] required check bogus_profile_rejected
    exit: exit 65
    stderr:
        sandbox-exec: unbound variable: a at <input string>, line 1, column 5

        Backtrace:
        <input string>:1:5:
        	a

[PASS] required check deny_default_denies_exec
    exit: exit 71
    stderr:
        sandbox-exec: execvp() of '/bin/echo' failed: Operation not permitted

[PASS] required check control_write_outside_root
    exit: exit 0

[PASS] required check write_denied_outside_root
    exit: exit 1
    stderr:
        mkdir: /private/var/folders/…/saya-sandbox-probe-outside-root-6618/denied-write: Operation not permitted

[PASS] required check write_allowed_in_root
    sandboxed mkdir inside fs_roots worked; the directory exists unsandboxed afterwards:
    exit: exit 0

[PASS] required check control_read_outside_roots
    exit: exit 0
    stdout: <contents of /private/etc/hosts>

[PASS] required check read_denied_outside_roots
    exit: exit 1
    stderr:
        cat: /private/etc/hosts: Operation not permitted

[PASS] optional check net_rule_variant_accepted
    chosen: tcp localhost exact
    REJECTED tcp raw host:
        exit: exit 65
        stderr:
            sandbox-exec: host must be * or localhost in network address

            Backtrace:
            <input string>:7:26:
            	(remote tcp "127.0.0.1:60986")

    ACCEPTED (profile compiles) tcp localhost exact: exit: exit 0
    ACCEPTED (profile compiles) ip localhost exact: exit: exit 0
    ACCEPTED (profile compiles) tcp any port: exit: exit 0

[PASS] required check control_net_connect
    exit: exit 0

[PASS] required check net_denied_without_allow
    exit: exit 1
    stderr:
        /bin/bash: connect: Operation not permitted
        /bin/bash: /dev/tcp/127.0.0.1/60986: Operation not permitted

[PASS] optional check net_allowed_with_rule
    rule kind tcp localhost exact: of 10 attempted connects to the allowed port, 10 exited cleanly and 10 landed on the listener; the seatbelt allow for net_allow on this host is therefore reliable (every connect landed).

[PASS] optional check net_other_port_denied
    rule kind tcp localhost exact: connecting to a port the rule does not name was denied in 5/5 attempts (port-exact enforcement).
```

(The `write_denied_outside_root` path and `net_denied_without_allow` port are
ephemeral per run; `…` marks elided temp-dir prefixes, and the
`/private/etc/hosts` contents printed by the control check are elided — the
byte-exact report is written to `$CARGO_TARGET_TMPDIR/sandbox-probe-report.txt`
on every run.)

## 3. What the probe measures, and why each check exists

Required checks (macOS) — the fail-closed verdict is true only when **all** of
these pass:

1. `sandbox_exec_present` — `/usr/bin/sandbox-exec` exists. Absence ⇒ recorded,
   not a failed build.
2. `bogus_profile_rejected` — negative control: `sandbox-exec -p 'not a sandbox
   profile at all' /bin/echo` must **fail**. If it ever succeeds, sandbox-exec
   is a no-op and every later check is meaningless.
3. `deny_default_denies_exec` — a `(deny default)`-only profile must refuse
   exec (measured: exit 71, "Operation not permitted"). This is the proof that
   deny-by-default bites.
4. `control_write_outside_root` / `write_denied_outside_root` — the same
   `mkdir` succeeds unsandboxed and is denied sandboxed, with **nothing on
   disk** afterwards. A deny with only a non-zero exit would be unattributed;
   the probe requires the EPERM text in stderr.
5. `write_allowed_in_root` — the sandboxed child creates a directory inside
   `fs_roots`; the directory exists afterwards, verified unsandboxed.
6. `control_read_outside_roots` / `read_denied_outside_roots` — same pattern
   for `cat /private/etc/hosts`.
7. `control_net_connect` / `net_denied_without_allow` — unsandboxed
   `/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/P'` connects to a probe listener;
   sandboxed, with no net rule, it is denied with EPERM (measured, and the
   EPERM evidence is required).

Optional checks record the selective-egress half:

- `net_rule_variant_accepted` — which `(remote tcp …)` forms compile, verbatim.
- `net_allowed_with_rule` — 10 connects to the allowed port; passes only if
  **all** exit cleanly **and** land on the listener (10/10 measured here).
- `net_other_port_denied` — 5 connects to a port the rule does not name; passes
  only if all are denied with EPERM (5/5 measured here).

Cross-platform: on Linux the probe measures the Landlock ABI version (raw
syscall, `libc::SYS_landlock_create_ruleset`), reads
`/sys/kernel/security/lsm` and `/proc/sys/user/max_user_namespaces`, and forks
a child that runs `unshare(CLONE_NEWUSER)` + `unshare(CLONE_NEWNET)`
(forked because `unshare(CLONE_NEWUSER)` is refused in multithreaded processes
— per unshare(2), **unverified**). Its `landlock_enforcement` check is
**recorded as failed by design** in this slice: proving a ruleset denies needs
the `landlock` dependency, which this spike must not add. Linux therefore
cannot be proven yet — fail closed, honestly. On Windows the probe records one
explicit failed check ("no OS sandbox primitive this design targets"), so U9
is a recorded result, not an absence; the 3-OS matrix stays green with the
runner absent there.

## 4. Measurements that shaped the drafts (commands + raw output)

### 4.1 The startup SIGABRT — an undocumented required allow

A plausible profile — `deny default` + `process-exec` on `/bin` + the
conventional dyld read allows (`/usr/lib`, `/System/Library`,
`/private/var/db/dyld`, and later `/System/Volumes/Preboot/Cryptexes`) —
compiled fine and then **every child died with SIGABRT (exit 134) before dyld
printed anything**, for echo, mkdir, cat, and bash alike. Ten repetitions each:

```console
$ /usr/bin/sandbox-exec -p '(version 1)
(deny default)
(allow process-exec (subpath "/bin"))
(allow file-read* (subpath "/usr/lib") (subpath "/System/Library") (subpath "/private/var/db/dyld"))' /bin/echo ok
exit=134          # 10/10 runs
```

Bisecting by allowed scope (each row run 10×; exit −6 = SIGABRT):

```text
no /-allow at all           : [-6 ×10]
file-read-data  (literal "/") : [0 ×10]
file-read-metadata (literal "/"): [-6 ×10]
file-ioctl      (literal "/") : [-6 ×10]
file-read* (subpath "/usr/lib") : [-6 ×10]
(allow file-read*)  — everything  : works
```

`(allow file-read-data (literal "/"))` is required and sufficient; `sandbox-exec
… -p '(allow file-read-attr …)'` is a parse error (`sandbox-exec: unbound
variable: file-read-attr`). **Why is unverified** — the child dies before dyld
prints anything; no crash report is filed; the macOS 26 dyld cache lives at
`/System/Volumes/Preboot/Cryptexes/OS/System/Library/dyld/` and needs **no**
explicit read allow (its mapping rides on `process-exec`, measured).

The M4-1 lesson, again: the plausible configuration did not do what the design
assumed, and only a measured child run revealed it.

### 4.2 Canonicalisation is not cosmetic

The tempdir path as returned by `mkdtemp` is `/var/folders/...`; the resolved
path is `/private/var/folders/...`. Seatbelt matches the resolved path:

```console
# profile with (subpath "/var/folders/…/sbx-fs-…") (non-canonical)
$ sandbox-exec -p '…' /bin/mkdir /var/folders/…/sbx-fs-…/in-root
mkdir: /var/folders/…/sbx-fs-…/in-root: Operation not permitted   # exit 1
# same profile with the canonicalised root
$ sandbox-exec -p '…' /bin/mkdir /private/var/folders/…/sbx-fs-…/in-root
exit 0; the directory exists afterwards
```

The generator must canonicalise every `fs_roots` entry before substitution —
the same canonicalise-then-match discipline as the workspace containment
(`saya-harness/src/workspace/contain.rs`).

A related measured detail: a child whose cwd is outside the roots gets
`shell-init: error retrieving current directory: getcwd: cannot access parent
directories: Operation not permitted` at startup — noisy, and avoidable by
pinning the child's cwd to a canonicalised root (the runner does exactly that:
cwd = run workspace, M5-4).

### 4.3 Network rule syntax

```console
$ sandbox-exec -p '…(allow network-outbound (remote tcp "127.0.0.1:60986"))' /bin/echo x
exit 65
sandbox-exec: host must be * or localhost in network address
$ sandbox-exec -p '…(allow network-outbound (remote tcp "example.com:443"))' /bin/echo x
exit 65
sandbox-exec: host must be * or localhost in network address
```

`localhost` (mapping policy host `127.0.0.1`), `ip "localhost:P"` and `*:P`
all compile and, measured with an accepting endpoint, all passed 10/10
connects. The exact form is preferred; the port is exact (5/5 EPERM on a port
the rule does not name).

**Measurement lesson recorded because it changed the analysis once already:**
a first measurement round reported the allow rules as flaky (1/10…1/30) or
blocking. That was **not** the sandbox: the endpoint listener had been bound
with `listen(1)` and never accepted, so after the first connection the backlog
was full and later connects died as TCP timeouts — indistinguishable from a
sandbox deny by exit code alone. The probe's rule — a deny only counts with
the literal `Operation not permitted` in stderr — is what caught it:
`Operation timed out` is not sandbox evidence. Any future egress test in this
repo must keep that rule.

### 4.4 Comment syntax and profile parsing

```console
$ sandbox-exec -f <profile> /bin/echo ok      # one comment form per profile
/* block */   -> exit 65 (rejected)
// line       -> exit 65 (rejected)
; semi        -> accepted, child runs
# hash        -> exit 65 (rejected)
```

`;` is the only comment syntax `sandbox-exec` accepts on this host, both as a
full line and trailing. `sandbox-exec` itself reports parse errors with exit 65
and a backtrace into the profile text, and exec denials with exit 71 — the
probe's negative control relies on that:

```console
$ sandbox-exec -p 'not a sandbox profile at all' /bin/echo saya-probe-bogus
exit 65
sandbox-exec: unbound variable: a at <input string>, line 1, column 5
```

### 4.5 `process-fork` is not free

A compound command measured exactly:

```console
$ sandbox-exec -p '…(deny default)…' /bin/bash -c 'sleep 0.5; exec 3<>/dev/tcp/…'
exit 128
/bin/bash: fork: Operation not permitted
```

Single-command children never needed it; any allowlisted program that forks
needs `(allow process-fork)` added only with a measured reason.

## 5. Draft verification (the macOS draft, filled and run)

`docs/sandbox-profile-macos.sb.draft` — two `fs_roots` (run dir + child temp)
and one `net_allow` entry, substituted and executed via
`sandbox-exec -f`:

```text
echo:                    exit 0, stdout "ok"
mkdir in run dir:        exit 0
mkdir in child temp:     exit 0
mkdir outside roots:     exit 1, "Operation not permitted", nothing on disk
cat /private/etc/hosts:  exit 1, "Operation not permitted"
connect to allowed port: exit 0, listener received "ping"
connect to other port:   exit 1, "Operation not permitted"
```

## 6. Unmeasured — stated plainly

- **Linux: UNMEASURED.** This spike ran on macOS. The Landlock ABI, the LSM
  list, and unprivileged user/network namespace creation are *code paths the
  probe will measure on a Linux host*, not measured facts. The Linux draft
  marks every non-macOS claim `[UNVERIFIED]` and cites only the kernel's own
  documentation as a source, never observation. What was verified without
  measuring: the probe file **compiles warning-free for Linux
  (`x86_64-unknown-linux-gnu`) and Windows (`x86_64-pc-windows-msvc`)** —
  checked by compiling it in a standalone scratch crate with the same
  `libc` gate, since the workspace's own tree cannot cross-build (`ring`).
  Running it on those platforms remains unmeasured.
- **Windows: UNMEASURED by construction** — there is nothing to measure; the
  probe records the explicit fail-closed result (U9).
- **Why the fork+unshare probe has not run on Linux:** it is compiled and
  shipped in the probe, but this host is macOS; the implementation slice runs
  it on Linux and amends the ADR with what it measures.
- Unmeasured for arbitrary *runner* programs: `process-fork`, `mach-lookup`,
  and dyld behaviour on macOS versions older than 26.6.2.

## 7. The `landlock` dependency — reported, not added

The Linux half will need the `landlock` crate (this slice deliberately did not
add it; `cargo audit` runs whenever it is added). Facts from the crates.io
sparse index (fetched 2026-09-11):

- Latest release: **0.4.7** (2026-07-27); `rust_version = "1.71"` — below this
  workspace's MSRV 1.88.
- Runtime dependencies: `libc ^0.2.186` (already in the tree at 0.2.189),
  `enumflags2 ^0.7`, `thiserror ^2.0` (already workspace-shared). Dev-only:
  `anyhow`, `lazy_static`, `strum`, `strum_macros`. No transitive risk surface
  beyond three small crates.
- It would be added under `[target.'cfg(target_os = "linux")'.dependencies]`
  with the `cargo audit` gate run at that time, per the plan's
  dependency-review rule.

## 8. Decision (draft — for the security reviewer)

Measured on macOS 26.6.2: the generated Seatbelt profile confines children
reliably — exec allowlisted paths only, filesystem access confined to
`fs_roots` with EPERM evidence for every measured denial, egress denied
without a rule and allowed port-exact with one. The macOS runner sandbox is
therefore **provable by startup probe, fail closed**. The measured constraint
is the host form: Seatbelt accepts only `localhost` and `*` — `net_allow`
entries must be loopback or port-exact wildcards; selective egress to a named
remote host is **not expressible** in this profile language on macOS 26.6.2.

On Linux, nothing is measured; the probe records Landlock ABI and namespace
availability only and its `landlock_enforcement` check is failed by design, so
**Linux fails closed today** until the implementation slice adds the
`landlock`-based canary probe on a real Linux host and the ADR is amended with
what it measures (the ADR-0003 pattern: pin first, decide after).

Entry conditions for the M5-3 implementation slice, restated:

1. `RunSandbox` policy construction refuses non-expressible `net_allow`
   entries per platform (macOS: `localhost`-mapped or `*` host only) — refuse
   at construction, never silently narrow.
2. The runner is registered only where `ProbeReport::proves_runner()` is true,
   re-probed at startup on every run; unproven ⇒ runner tool absent and plans
   requesting the runner scope refused.
3. The profile generator canonicalises `fs_roots` before substitution
   (measured §4.2) and is injection-tested: a path containing `)` or a newline
   must not be able to alter the profile language.
4. The escape battery (plan §12.2) re-runs per macOS upgrade — Apple
   disclaims this interface; measured behaviour here is not a contract.

## 9. The three things most likely to make this fail in production

1. **The macOS security property rests on an interface Apple explicitly
   disclaims, whose behaviour already shifted under us during the spike.**
   `sandbox-exec` is deprecated, undocumented, and versioned only by Apple's
   discretion — and within one afternoon of measuring, this host produced two
   behaviours no documentation predicted: children SIGABRT without an
   undocumented `file-read-data` allow on `/` (§4.1), and the network filter
   refuses any host but `localhost`/`*`. Nothing binds Apple to keep either
   behaviour, and nothing tells us when it changes except probes and children
   failing at customer machines. The mitigation is structural, not
   reassuring: probe at every startup, fail closed, and treat a macOS upgrade
   (which CI will not see first) as a potential silent loss of the runner
   until the probe is re-run there.

2. **The Linux egress choice does not compose, and it is unmeasured.** The
   plan's Linux shape is Landlock for filesystem plus an unprivileged
   user+network namespace for egress. As documented (unverified): a fresh
   netns has **no** network at all, so a non-empty `net_allow` cannot be
   honoured inside one; and Landlock's network rules (kernel ABI ≥ 4, i.e.
   ≥ 6.7) restrict TCP **by port only** — there is no host dimension — so
   Landlock alone enforces `(*, port)`, not `(host, port)`. Honouring
   `net_allow (host, port)` therefore requires a proxy component the plan does
   not have, and on the LTS kernels most enterprise users run, Landlock
   network enforcement does not exist at all — the runner fails closed there,
   meaning the feature is absent exactly where sandboxing matters most. The
   spike could not choose between netns-only, Landlock-ports, or proxy from
   this host; the implementation slice must measure ABI reality on the target
   distributions first, and the reviewer must treat that as the decision point
   the plan flagged as highest-variance.

3. **Probe-time proof is not run-time enforcement, and the profile is built
   from attacker-influenced strings.** The probe proves availability at
   startup; the runner then constructs profiles from `fs_roots` paths and
   `net_allow` values that a plan or filesystem can influence. Seatbelt
   profiles are arbitrary profile-language text (measured: `sandbox-exec`
   parses whatever it is given), so a run directory named to embed
   `)` + newline is a profile-language injection unless the generator quotes
   and canonicalises defensively — and §4.2 shows seatbelt silently matching
   nothing when paths are not canonicalised, a bug that fails *open* on the
   allow side (child writes escape confinement only if the deny rules still
   cover the unresolved path — measured: they did, but that was luck of the
   resolved path, not a designed property). Separately, container and
   hardening environments (Docker seccomp, hardened macOS MDM profiles,
   `unprivileged_userns_clone=0`) can pass the probe at one moment and change
   under the run; the probe's re-run at startup and the runner's refusal to
   start unproven are the only guard, and any long-lived run outlives the
   evidence it registered with.

## 10. Sign-off (M5-3's blocking condition)

**Security reviewer sign-off required before the runner merges** (per U2:
profile drafts + this spike note are the review deliverables). Sign-off slot,
per the ADR-0003 shape:

- [ ] Reviewer name / date: ______
- [ ] Accepts the macOS measured constraint (host form `localhost`/`*` only)
      and its consequence for `net_allow` expressiveness.
- [ ] Accepts Linux fail-closed-until-measured, and the entry conditions in §8.
- [ ] Accepts the probe's deny-evidence discipline (EPERM text required; TCP
      timeout is not a denial) as the standing measurement rule for egress.