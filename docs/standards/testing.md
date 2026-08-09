# Testing standard

Tests are the contract. They are how an agent proves a change is correct without a
human re-reading every line. **Test behavior, not implementation.**

## Test-driven by default

Every behavior change **starts with a failing test**:

1. **Red** — write the smallest test that captures the new behavior (or reproduces the
   bug). Run it; watch it fail *for the reason you expect*.
2. **Green** — write the least code that makes it pass. Don't gold-plate.
3. **Refactor** — clean up under a green suite. Names, dedup, file-size splits.

`/tdd` drives this loop. A PR that adds behavior without a test that would fail on
`main` is incomplete.

## The runner

We use **[cargo-nextest](https://nexte.st)** — faster, clearer output, per-test
isolation. Doctests run separately (nextest doesn't execute them).

```bash
cargo nextest run --workspace --locked   # the suite
cargo test --workspace --doc --locked     # doctests
```

Config lives in [`.config/nextest.toml`](../../.config/nextest.toml) (`default` for
local, `ci` for CI: no fast-fail, retries for known-flaky live tests, JUnit output).
`cargo test` still works for anyone without nextest installed.

## The taxonomy — pick the smallest kind that proves the behavior

| Kind | Where | Use for | Example in repo |
| --- | --- | --- | --- |
| **Unit** | inline `#[test]` / `*_tests.rs` beside the code | pure logic, one function | `connection/build_tests.rs`, `config/init_tests.rs` |
| **Integration** | `crates/<c>/tests/*.rs` | a crate's public surface end-to-end | `saya-cli/tests/mvp/*`, `saya-config/tests/*` |
| **Contract** | `tests/` in the owning crate | a capability behaves identically across backends | `saya-connectors/tests/{contract,read_only_contract}.rs` |
| **Live** | `tests/*_live.rs`, **env-gated** | real DB/network behavior | `postgres_live.rs`, `mysql_live.rs` |
| **Snapshot** | `insta` | rendered / serialized output that's tedious to assert by hand | render, TUI text, JSON events |
| **Property** | `proptest` | invariants over a large input space (parsers, classifiers) | read-only SQL safety |

### Live tests are opt-in

Gate them on a `SAYA_TEST_*` env var (e.g. `SAYA_TEST_POSTGRES_URL`) and **skip
cleanly when unset** — never fail a developer's machine for lacking a database. CI
provides the databases as services (see `ci.yml`).

### Snapshot tests (`insta`)

For output where the *shape* is the assertion — rendered text, JSON events, schema
dumps. Write the assertion, run once, review the generated snapshot deliberately:

```rust
insta::assert_yaml_snapshot!(render_event(&event, RenderFormat::Json));
```

```bash
cargo insta test --review   # accept/reject changed snapshots
```

Review every snapshot change like code — a diff you didn't intend is a regression, not
a rubber stamp. Snapshots must be **deterministic**: no timestamps, absolute paths, or
unordered maps. Redact volatile fields with insta filters.

### Property tests (`proptest`)

For invariants that must hold over *all* inputs, not the handful you'd pick by hand.
The read-only safety layer is the canonical case: *no statement it accepts may mutate
data*. Keep the property pure and fast; shrink to a minimal counterexample on failure.

## Fixtures & data — never commit secrets

- **No real credentials, `.env` files, session directories, or raw query results** in
  fixtures or snapshots. Use redacted fixtures and explicit secret *references*
  ([security.md](security.md)).
- Fixtures are small and readable. Prefer constructing values in-test
  (`QueryResult { … }`) over checked-in blobs when it's clearer.

## Coverage expectations (by layer, not a percentage)

- **`saya-types`, `saya-config`** — unit + integration; every public function and every
  error/diagnostic path exercised.
- **`saya-connectors`** — contract tests for every backend; **security tests for the
  safety layer are mandatory** and property-based; live tests behind env gates.
- **`saya-store`, `saya-agent`** — integration over the public surface; provider wire
  formats covered by unit/snapshot tests (`providers/*`).
- **`saya-cli`** — integration over commands + slash parsing; snapshot tests for
  rendering. TUI logic that can be exercised headlessly should be.

A bug fix adds the regression test that would have caught it. A new capability ships
with contract + security tests before it's advertised.
