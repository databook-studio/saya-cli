# `run_program` escape report (M5-4)

- **Status:** measured findings from the M5-4 escape battery, in the ADR-0003
  shape (measured facts → what changed → sign-off question). Companion to
  `docs/sandbox-spike.md`; that document's discipline — a deny counts only
  with evidence, and a plausible configuration is not a fact — applies here.
- **Date:** 2026-09-11
- **Host:** the same macOS 26.6.2 (Darwin 25.6.0, arm64) host the M5-3 spike
  measured. Linux and Windows remain UNMEASURED; the runner is absent there
  by the fail-closed probe decision, and the battery's refusal half is
  written to run (typed-refusal tests) on any platform CI covers.
- **What this is not:** M5-5 (nested saya) and M5-7 (credential-isolation
  E2E) were deliberately not implemented.

## 1. Escapes the design did not anticipate

### 1.1 A Rust child dies SIGABRT before `main` — the deny was EINVAL, not EPERM

**The escape.** The battery's helper is a Rust binary. Under the generated
profile — `(deny default)` with every measured allow — it died before `main`
with SIGABRT (exit 134):

```text
thread 'main' panicked at std/src/sys/pal/unix/stack_overflow.rs:539:13:
failed to allocate a guard page: Invalid argument (os error 22)
fatal runtime error: initialization or cleanup bug, aborting
```

**The bisect.** C binaries under the identical profile ran fine (so the
spike's canaries could never have caught this). `(allow default)` made the
Rust child work. Adding candidate operation classes one at a time:

```text
deny default + file-read* + file-write* + process-exec:
    SIGABRT, "failed to allocate a guard page: Invalid argument"
  + process-info*:     SIGABRT
  + process-fork:      SIGABRT
  + file-ioctl:        SIGABRT
  + mach-lookup + ipc*: SIGABRT
  + sysctl-read:       exit 0
```

**Mechanism (consistent with the observation, mechanism unverified):** a Rust
child queries a sysctl at runtime init — the `sysconf(_SC_PAGESIZE)` its
guard-page setup performs maps to a sysctl read on this kernel — and the
denied query surfaces as `EINVAL`, which std treats as fatal. This is the
**first measured denial on this host that does not carry the literal
`Operation not permitted`**. The spike's standing rule ("a deny only counts
with a literal EPERM in stderr; a timeout is not sandbox evidence") needs its
complement recorded: **a denial may also surface as EINVAL from the child's
own runtime, and only a working allow proves the cause.**

**What changed.** The generated macOS profile carries `(allow sysctl-read)`
with the measured reason in its comment, next to the §4.1 dyld allow it
resembles (both are undocumented required allows found only by running real
children). The battery runs a Rust child through the real generator output on
every run, so a future macOS version that breaks this fails a named test
instead of a customer run.

**Sign-off question for the reviewer:** `sysctl-read` is the whole class of
kernel info leaks the profile language can express — narrower filtering (a
specific sysctl key) was not attempted because the failing query is inside
std and is not a documented interface; refusing the allow means no Rust
program can ever be allowlisted on macOS. Accepted as the trade?

### 1.2 The child that exits and leaves the grandchild behind

**The escape.** Red test 4 as written ("a timeout kills a daemonizing
grandchild") assumes the child is still alive when the timeout fires. The
battery also tried the shape that assumption misses: the child forks a
detached grandchild and **exits 0**. The timeout never fires, the runner's
wait completes successfully — and the grandchild survives the entire run,
holding the pipes and the process group, with the tool reporting success.

**What changed.** The runner now sweeps the child's process group after the
child is reaped: signal 0 probes the group, live members are SIGKILLed, and
the outcome carries `killed_orphans: true` — reported, never silent. The
sweep runs before the output pumps are joined, because the orphaned
grandchild holds the pipe write ends and would deadlock the pump joins. The
group is the child's own (`process_group(0)`), so no unrelated process is
reachable.

**Honesty note.** A program that daemonizes legitimate work (a dev server
for a later step) will have that work killed and reported. That is the
containment contract — the runner leaves no orphans — and the report says
so. If a workload needs a daemon, that is a plan-shaped decision, not
something `run_program` grants silently.

### 1.3 A program directory inside the workspace lets a child rewrite its allowlist

**The escape.** `RunSandbox::prepare(program_dir)` allowlists exec of the
program directory's subpath, and `fs_roots` governs writes. If the program
directory sits **inside** a root, a child can overwrite its sibling binaries
— the next allowlisted program to run is whatever the previous child wrote.

**What changed.** The battery stages the program directory **outside**
`fs_roots` (exec-allowed, not write-allowed). Nothing in the runner's
production code can enforce that — the composition root builds the policy —
so this is a contract the next slice (composition root wiring) must keep.
The battery's provision is the reference shape.

### 1.4 Symlinked programs: refused by design where the sandbox would have refused by luck

**The escape.** A symlink inside the program directory pointing outward:
Seatbelt matches the *resolved* path (measured, spike §4.2), so the exec
would be denied by the sandbox — but nothing in the tool's validation
guaranteed that; it worked out that way on this host. The battery refuses
symlinks at validation (fail closed by design), alongside scripts (shebang)
and non-regular files.

### 1.5 Interpreter trampolines beyond `bash`/`sh`

**The escape.** The red tests name `bash` and `sh`. `env`, `python3`, `awk`,
`perl`, `osascript`, `script` and friends are equally able to spawn
arbitrary children with arbitrary argv — the escape the allowlist cannot
survive regardless of the sandbox. The refusal list
(`saya_types::is_refused_runner_program`) covers them and is shared by
config resolution (refuse at resolve time) and the runner (refuse even when
a hand-built scope carries the name), so the two layers cannot disagree.

### 1.6 The measured reason's own text is a profile substitution

**The escape.** `RunSandbox::with_process_fork(reason)` substitutes the
reason into a generated profile comment. A reason containing an em dash, a
parenthesis, or a semicolon is refused by the same conservative text class
every other profile substitution carries. The battery's first reason string
was refused — correctly. Anticipated in mechanism, discovered in practice:
the reason is untrusted-adjacent text and is treated like a path.

## 2. What the battery confirmed about the design's own claims

- **Typed argv holds.** `;`, `$()`, backticks, spaces, and newlines each
  arrived byte-exact as one argv element (asserted on the child's own dump
  of argc + per-element hex).
- **The four named refusals hold**, each as its own named test:
  `bash -c`, `sh -c`, a wrapper script, and an absolute path to a lookalike.
- **The timeout kills the group**, including a forked-and-detached
  grandchild, and a requested timeout narrows but never widens the declared
  default.
- **The child's environment is built, not inherited**: a planted database
  credential was absent from the child's actual environment (the child
  printed its env; the assertion reads what the child saw).
- **The four credential conditions hold as designed**: declared-only,
  sandbox-only (no `RunnerSpawn`, no tool), references-only (unresolvable
  reference fails the call before anything spawns), redact-always (the
  planted `password=`, bearer header, and URL userinfo were scrubbed in both
  the model result and the disk record).
- **4 GiB through the ring stayed at the cap** (64 KiB per stream), tail
  retained, truncation and drop accounted; a real 1 MiB flood through the
  sandboxed runner reported the same way.
- **Exit code is data**: a non-zero exit is an `Ok` outcome.
- **Cancellation uses the same killpg path** and reports `cancelled`, never
  `killed_by_timeout`.

## 3. Deviations and amendments, stated plainly

1. **A fifth production file.** `runner/error.rs` joins the four named
   files: the typed error surface alone would have pushed `mod.rs` past the
   250-line hard rule. No `#[allow(...)]` anywhere; every file ≤ 250 lines.
2. **Additive `ToolError::Runner` variant** in `saya-agent` (detail text
   carrier) — the runner's typed refusals reach the model with their own
   wording instead of a borrowed variant's.
3. **`RunnerSpawn` gained `Clone`.** The configuration is immutable; cloning
   grants nothing the proven arm did not hand out. The composition root
   still only obtains one from the proven arm of `prepare`.
4. **`RunnerScope::new` was hardened** (saya-types): it previously accepted
   path-shaped "program names" (`/bin/echo` passed the name-shape check).
   Scopes now demand bare names (`is_bare_name`), matching what every layer
   below refuses.
5. **The probe carries the fork grant through** (`probe_policy`): the
   startup probe measures the profile the runner would actually generate —
   a granted `process-fork` and its reason included — never a lookalike.
6. **The deny-evidence rule is amended by measurement**: sysctl denials
   surface as EINVAL, not EPERM. The spike's rule stands for egress and
   file operations; a failing required check needs neither if the allow
   provably fixes the child.

## 4. Measured-constraint restatements (unchanged from the spike, honoured)

- `process-fork` is granted only by explicit opt-in with a measured reason;
  the reason is emitted as the profile's own comment.
- Egress enforcement is the **port** of `net_allow`; the host component is
  the M3-1 fetch policy's job, in-process. Nothing in the runner or its
  tool description implies otherwise.
- Seatbelt is Apple-disclaimed and versioned by their discretion; the
  battery re-runs per macOS upgrade, and the runner fails closed wherever
  the startup probe does not prove the host.

## 5. Sign-off

- [ ] Reviewer accepts §1.1's trade: `sysctl-read` is granted to every
      runner child on macOS because Rust-built allowlisted programs cannot
      start without it (measured; C binaries unaffected).
- [ ] Reviewer accepts the post-exit group sweep (§1.2) and its honesty
      cost: daemonized grandchildren are killed and reported, not preserved.
- [ ] Reviewer accepts the program-directory-outside-roots composition
      contract (§1.3) as a binding requirement for the composition root.
- [ ] Reviewer accepts the shared interpreter refusal list (§1.5) as the
      one source of truth for both config and runner.