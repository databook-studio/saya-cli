# Contributing

SAYA CLI is developed as a small Rust workspace. Before opening a change:

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Begin behavior changes with failing tests. Keep public contracts in the
appropriate crate and keep CLI presentation in `saya-cli`. New production Rust
files should stay below 150 lines and must not exceed 250 lines. Do not add
Tauri, React, licensing, or desktop application state.

Never commit secrets, `.env` files, session directories, private keys, or raw
database results. Use redacted fixtures and explicit secret references. New
connectors and providers must remain behind honest capability boundaries until
contract, integration, and security tests exist.

Use conventional commits, explain alpha limitations in user-facing changes,
and include documentation for new flags or configuration fields.
