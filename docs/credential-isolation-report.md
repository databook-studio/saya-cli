# Credential-isolation E2E report (M5-7)

- **Status:** measured findings from the M5-7 credential-isolation battery
  (`crates/saya-harness/tests/credential_isolation.rs`, 6 tests), in the
  ADR-0003 shape (measured facts → what changed → sign-off question).
  Companion to `docs/run-program-escape-report.md` (M5-4); that document's
  discipline — a deny counts only with evidence, and a plausible
  configuration is not a fact — applies here.
- **Date:** 2026-09-11
- **Host:** the same macOS 26.6.2 (Darwin 25.6.0, arm64) host the M5-3/M5-4
  measurements ran on. The battery is macOS-gated like the escape battery:
  the startup probe must prove the sandbox or the tests compile to nothing
  rather than pass vacuously. Linux and Windows remain UNMEASURED; the
  runner is absent there by the fail-closed decision.
- **What this is:** the end-to-end credential path — planted-secret sweeps
  through the episode loop, the engine sink, the journal, the store, the
  generated configs, and the runner's disk records. It is not the spawn
  level; `tests/runner.rs` owns that and was read, not duplicated.
- **Scope note:** the composition root does not yet attach the runner tool
  to a run's toolset (the run tool universe is `DatabaseTools` only), and
  the nested-saya wiring (M5-5) was deliberately not implemented. The
  battery therefore drives the end-to-end path through the engine the
  composition root will use: `EpisodeDriver` → tool execution → capture →
  tool result to the provider → journal/store mirrors.

## 1. The hole the battery found — fixed

### 1.1 The exact injected value survived capture when echoed bare

**The escape.** The battery's exfiltration test has the child echo the
credential it legitimately received, under the env name the generated config
actually binds: `SAYA_RUN_EP_ORCHESTRATOR=planted-orchestrator-…`. `redact()`
is pattern-based — it needs a marker (`password=`, an `Authorization:`-style
header, URL userinfo, `api_key=`) to recognise a secret — and an
environment dump names the value without any marker. The pre-fix capture
output, measured:

```text
password=[redacted]
https://[redacted]@db.example.com/x
SAYA_RUN_EP_ORCHESTRATOR=planted-orchestrator-7Qk2mVn9xR4e2e   ← unredacted
```

The raw value reached **both** destinations — the model lane (the `tool`
message the loop hands the provider) and the disk lane (the record the
runner persists into the run workspace) — and the same leak appeared on the
failure path (three bounded attempts, run paused mid-flight, the record
files carrying the value). The M5-4 battery did not catch this because its
test credential was deliberately named `SAYA_PROBE_ENDPOINT_API_KEY`, whose
name carries the `api_key=` marker — a test choice, not a production
guarantee.

**What changed (minimal).** The capture boundary (`output::capture`) now
takes the list of values the runner injected into that child's environment
(`env::inject` returns them) and scrubs every occurrence with the same
`[redacted]` marker, **before** the pattern pass — the pattern pass cannot
fragment an exact value, so the registry pass runs first. The boundary keeps
its "unconditional by construction" property: `capture` is still the only
function captured bytes may leave through, and the secrets it must scrub are
now part of its signature. Empty values are skipped (an empty pattern would
destroy the capture); a value split across the ring's cap boundary cannot be
matched as one string — stated in the code, not hidden.

**Sign-off question for the reviewer:** the value registry is exact-string
matching over values the runner itself resolved. It cannot catch a
credential the child obtained some other way (a value the child derived, or
a second credential smuggled through argv) — that remains pattern redaction's
job, with its known limits. Accepted as the trade?

## 2. What the battery proved (end to end, macOS)

- **The full sweep holds.** A resolved credential appears in exactly one
  place: the child's environment — proven by a digest the child computes of
  its own env value (the raw value is never printed by the test, so the
  sweep's only expected hit is the environment itself, which no byte scan
  can reach). The whole run directory is scanned recursively (generated
  configs, journal, runner records, workspace), plus the store database with
  its `-wal`/`-shm` sidecars, plus the tool result the model received. Zero
  hits outside the environment; the digest matches.
- **Both redaction lanes hold, separately.** The model lane is the `tool`
  message the provider actually received; the disk lane is the record read
  back from disk. Both scrub the echoed credential and the recognised
  shapes, both show `[redacted]`, and the env-name shape survives with its
  value gone — proving the child really echoed the credential.
- **Declared removed ⇒ no credential**, through the episode loop: an empty
  step credential list leaves the run-scoped variable `UNSET` in the child's
  own report.
- **References-only removed ⇒ nothing spawns, the model is told.** A
  declared credential whose reference cannot resolve fails the call; the
  failure reaches the model as the tool result naming the credential, and
  the runner produces no record (no child ran).
- **Sandboxed** — redundant at this level, deliberately not duplicated: the
  condition is structural (no proven `RunnerSpawn`, no tool, no credential
  list to attach), and there is no end-to-end path that could carry a
  credential without it. `tests/runner.rs`'s
  `an_unsandboxed_host_has_no_runner_to_inject_through` and
  `the_runner_is_absent_where_the_probe_did_not_prove` are the whole proof;
  the E2E battery constructs every tool through the proven arm and cannot do
  otherwise.
- **Role separation holds within one run.** Step 0 (reviewer role) sees
  `SAYA_RUN_EP_REVIEWER` set and `SAYA_RUN_EP_ORCHESTRATOR` unset; step 1
  (orchestrator role) the other way round; the generated config binds each
  role to its own run-scoped env variable and echoes neither the original
  reference nor any value.
- **The failure path scrubs too.** A run that fails mid-flight (provider
  fails after the child ran; the bounded retry spends all three attempts;
  the run pauses) leaves a journal whose nine events tell exactly the
  bounded-retry story and a store whose run row is `Paused` and step row
  `Failed` — and neither, in bytes (db + WAL sidecars) or rows, carries any
  credential material. Every attempt's record shows the redaction.

## 3. Claims this battery could NOT prove, and why

1. **The nested-saya leg.** The design's claim is scoped to a nested `saya`
   carrying a credential to a child. Nested `saya run` exists only at the
   CLI session layer (`/run` spawns the binary with inherited env), and the
   composition root does not yet attach the runner tool to a run's toolset —
   so no nested child can hold a runner credential today. There is no path
   to exercise; testing it would mean building the missing wiring, which
   this slice forbids (tests and evidence only). **Unproven; vacuous by
   absence of the feature, not by absence of a test.**
2. **"Declared in the approved plan" as a document property.** The E2E
   battery proves the seam the code exposes: the tool's credential list is
   the only injection source, and an empty list injects nothing through the
   full loop. The derivation *plan → credential list* belongs to the
   composition root, which does not wire the runner tool yet — so the
   approved-plan document itself cannot be exercised end to end. **Proven at
   the tool seam; the plan-derived wiring remains unproven until the
   composition-root slice lands.**
3. **Non-macOS hosts.** The startup probe proves the sandbox on macOS only;
   elsewhere the runner is absent and these tests compile to nothing. The
   battery's preconditions are false there by the fail-closed decision, so
   there is nothing to prove — but the macOS findings transfer to no other
   host without re-running.
4. **A value split across the ring's cap boundary** cannot be scrubbed as
   one string (the registry pass is exact-string). Stated in `output.rs`;
   not tested, because constructing it would require a credential value
   longer than the 64 KiB cap straddling the boundary — a shape the value
   registry cannot fix by construction, only note.

## 4. Deviations and amendments, stated plainly

1. **One production change, minimal, forced by measurement:** the capture
   boundary gained the value-registry pass (`output::scrub_secrets`),
   `capture` takes the injected values, and `env::inject` returns the
   resolved values it just resolved. No new file, no new dependency, no
   `#[allow(...)]`; `redact()` itself is untouched. Three doc comments
   amended (runner `mod.rs`, `env.rs` condition 4, `output.rs` module +
   capture docs) and one unit test added in `output.rs`.
2. The battery reuses the escape battery's discipline (one compiled helper,
   one proven provision per test, byte-window sweeps) and its shape for the
   four-conditions matrix, covering the end-to-end variant where one exists
   and naming the redundancy where it does not.

## 4. Sign-off

- [ ] Reviewer accepts §1.1's fix: the capture boundary scrubs the exact
      injected credential values (value registry before pattern pass), and
      the stated residual risks (second-source values, cap-boundary split)
      are the accepted trade.
- [ ] Reviewer accepts the nested-saya leg as unproven-by-absence (§3.1)
      until the composition root wires the runner tool into the run toolset.