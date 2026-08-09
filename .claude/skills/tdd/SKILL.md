---
name: tdd
description: Drive a change in the saya workspace test-first — red → green → refactor with cargo-nextest. Use when implementing a planned change, fixing a bug, or whenever the user asks to work test-driven / TDD / "write the test first". Follows docs/standards/testing.md.
---

# Test-driven development

The second phase of saya's golden path. **Every behavior change starts with a failing
test.** Full detail: [`docs/standards/testing.md`](../../../docs/standards/testing.md).

## The loop

Repeat per increment of behavior:

1. **Red** — write the smallest test that captures the new behavior (or reproduces the
   bug). Pick the right kind and location:

   | Kind | Location | For |
   | --- | --- | --- |
   | unit | inline `#[test]` / `*_tests.rs` | one pure function |
   | integration | `crates/<c>/tests/*.rs` | a crate's public surface |
   | contract | `tests/` in owning crate | same behavior across backends |
   | live | `tests/*_live.rs`, `SAYA_TEST_*`-gated | real DB/network |
   | snapshot (`insta`) | `tests/` | rendered / serialized output |
   | property (`proptest`) | `tests/` | invariants over many inputs |

   Run it and **confirm it fails for the expected reason**:
   ```bash
   cargo nextest run -p <crate> <test-name>     # or: cargo test -p <crate> <test-name>
   ```

2. **Green** — write the least code that makes it pass. No gold-plating, no unrequested
   features. Keep files within the size cap and code in the right crate
   ([architecture.md](../../../docs/standards/architecture.md)).

3. **Refactor** — clean up under a green suite: names, dedup, split oversized files.

## Notes

- **Snapshots**: first run creates them — review deliberately, then re-run clean.
  ```bash
  INSTA_UPDATE=always cargo test -p <crate> --test <file>   # generate
  cargo insta review                                        # or accept/reject interactively
  ```
  A snapshot diff you didn't intend is a regression, not a rubber stamp.
- **Safety-layer changes require property/security tests** — never widen the read-only
  allow-list without a test proving the new construct can't mutate
  ([security.md](../../../docs/standards/security.md)).
- Keep the suite green between commits. Doctests run via `cargo test --workspace --doc`.
- When the change is implemented and green, hand off to `/rust-review`.
