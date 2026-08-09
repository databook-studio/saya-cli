# Security standard

saya connects to production databases and to LLM providers on the user's behalf. The
trust model is simple: **saya reads, it does not write, and it never leaks secrets.**
These rules are non-negotiable — a violation is a release blocker, not a style nit.

## Read-only by default

- **All SQL passes through the safety layer** in
  `crates/saya-connectors/src/safety/` (`prepare_{postgres,mysql,duckdb,snowflake}_sql`).
  It parses with `sqlparser`, rejects anything that isn't a single read-only statement,
  denies dangerous functions/relations (file access, sequences, stage/system calls),
  and caps result rows. Connectors must route through it — **never** execute
  caller-supplied SQL directly.
- **Never add a bypass** ("trusted" flag, raw-exec path, admin mode). If a genuine new
  read-only construct is wrongly rejected, widen the allow-list *with a property test*
  proving the construct can't mutate — don't punch a hole.
- Approval modes (`ask` / `read-only` / `never`) gate *tool* execution in the agent
  loop; they are a second layer, not a replacement for the SQL guard.

## Secrets never enter the tree

- **No `.env` files, private keys, credentials, session directories, or raw database
  results** in commits, fixtures, snapshots, or logs. `.gitignore` and review enforce
  this; assume anything committed is public forever.
- Config carries **secret references**, not secret values — resolved at runtime from
  the environment / files (`saya-config`: `secret.rs`, `env_file.rs`). A config file in
  the repo shows the *reference*, never the value.
- Store and audit layers **redact** before persisting (`saya-store`: `redaction.rs`).
  Any new persisted field that could carry user data goes through redaction — add a
  test asserting the secret doesn't survive a round-trip.

## Honest capabilities

- A connector/provider advertises a capability **only once contract, integration, and
  security tests cover it.** No "works but untested" surface reaches users.
- Fail closed: on any doubt about whether an operation is read-only or a value is
  sensitive, reject / redact. Surface a clear typed error, don't guess.
- User-facing text and errors must not echo secrets — `saya-cli`'s `render` layer is
  the last line; keep secret values out of `TerminalEvent` payloads.

## Dependencies

- `cargo audit --deny warnings` runs in CI (`.github/workflows/audit.yml`, weekly +
  on manifest changes) against `.cargo/audit.toml`. A new advisory fails the build.
- Adding a dependency is a security decision: prefer the workspace-pinned set, justify
  new crates in the PR, and keep `default-features = false` where we already do
  (`reqwest`, `sqlx`) to avoid pulling native-TLS / unused backends.

## If you find a vulnerability

Follow [`SECURITY.md`](../../SECURITY.md) — private disclosure, not a public issue.
