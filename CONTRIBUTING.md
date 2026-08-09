# Contributing

SAYA CLI is a small Rust workspace (edition 2024, MSRV 1.88). The engineering
standards live in [`docs/standards/`](docs/standards/README.md); the one-page overview
for both people and AI agents is [`AGENTS.md`](AGENTS.md). **Read those first** — this
file is just the quick start.

## The loop

Plan → test-drive → review. Every behavior change **starts with a failing test**, and
the diff is reviewed against the standards before it's committed. See
[`docs/standards/workflow.md`](docs/standards/workflow.md).

## Local gate (run before you push)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked      # or: cargo test --workspace --locked
cargo test --workspace --doc --locked
cargo audit --deny warnings                 # when dependencies changed
```

`cargo-nextest` is the standard runner ([testing.md](docs/standards/testing.md)); plain
`cargo test` works too.

## The essentials (full detail in the standards)

- **Read-only by default.** All SQL goes through the safety layer in `saya-connectors`.
  Never add a bypass. New connectors/providers stay behind honest capability boundaries
  until contract, integration, and security tests exist.
  ([security.md](docs/standards/security.md))
- **No secrets in the tree** — never commit secrets, `.env` files, session directories,
  private keys, or raw database results. Use redacted fixtures and secret *references*.
- **Contracts in their crate; presentation only in `saya-cli`.** New production `.rs`
  files stay ≤ 150 lines (soft) and must not exceed 250.
  ([architecture.md](docs/standards/architecture.md))
- **Conventional commits**; document new flags/config and note alpha limitations in
  user-facing changes. Do not add Tauri, React, licensing, or desktop app state.

Working with an AI agent? The skills `/plan-first`, `/tdd`, `/rust-review`, and
`/rust-standards` automate the loop above.
