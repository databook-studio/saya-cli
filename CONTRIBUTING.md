# Contributing

SAYA CLI is a small Rust workspace (edition 2024, MSRV 1.88). This guide is the
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
- **Contracts in their crate; presentation only in `saya-cli`.** New production
  `.rs` files stay ≤ 150 lines (soft) and must not exceed 250.
- **Conventional commits**; document new flags/config and note alpha limitations
  in user-facing changes. Do not add Tauri, React, licensing, or desktop app
  state.
