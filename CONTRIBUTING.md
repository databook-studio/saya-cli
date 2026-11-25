# Contributing

SAYA CLI is a small Rust workspace (edition 2024, MSRV 1.89). This guide is the
quick start; the sections below are self-contained.

## The loop

Plan → test-drive → review. Every behavior change **starts with a failing test**,
and the diff is reviewed before it's committed.

## Local gate (run before you push)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked      # or: cargo test --workspace --locked
cargo test --workspace --doc --locked
cargo audit --deny warnings                 # when dependencies changed
```

`cargo-nextest` is the standard runner; plain `cargo test` works too.

## The essentials

- **Read-only by default.** All SQL goes through the safety layer in
  `saya-connectors`. Never add a bypass. New connectors/providers stay behind
  honest capability boundaries until contract, integration, and security tests
  exist.
- **No secrets in the tree** — never commit secrets, `.env` files, session
  directories, private keys, or raw database results. Use redacted fixtures and
  secret *references*.
- **Source file length.** Keep maintained Rust, Python, shell, and Ruby source,
  tests, and test support near 250 physical lines. Files at 300 lines or more
  require full-body review recorded against the exact source digest before
  integration. No maintained source file may exceed 400 lines.
- **Conventional commits**; document new flags/config and note alpha limitations
  in user-facing changes. Do not add Tauri, React, licensing, or desktop app
  state.
- **Performance budgets.** A tool is advertised on demand, its description stays
  short, and a tool added later is appended at the end of the list — unused tool
  text and long prompts slow every turn and cost correctness. The ceilings are
  tests, not suggestions: request budgets per surface in
  `crates/saya-cli/tests/run_core_parity/budget.json`, the always-on tool lists
  and description/section caps in `crates/saya-cli/src/perf_guard.rs`, and the
  timing bars in `scripts/perf-compare.sh`, run by the `perf-compare` CI job:
  the first message within control + 10 %, the steady turn overhead judged
  against the control measured in the same session (paired by run index), and
  idle CPU, draws and disk. The absolute steady-overhead rows (p50 ≤ 50 ms,
  p95 ≤ 120 ms) are reported beside the paired verdict, not asserted. Raising
  a bar is a reviewed change, with the reason recorded beside it.
