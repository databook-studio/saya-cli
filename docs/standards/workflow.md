# Workflow standard

Plan → test-drive → review → commit. Same loop for humans and agents; the skills
(`/plan-first`, `/tdd`, `/rust-review`, `/rust-standards`) automate the phases.

## 1. Plan before you edit

For anything beyond a typo or a one-line fix, write the plan down first:

- **Scope** — what changes, what explicitly does not.
- **Crates touched** and why (respecting the dependency direction,
  [architecture.md](architecture.md)).
- **Test list** — the tests you'll add/change and which kind each is
  ([testing.md](testing.md)). The plan names the *red* tests before code exists.
- **Risks & rollback** — what could break, how it's reverted.

`/plan-first` produces this. Don't touch code until the plan is written; for a
user-facing feature, get sign-off on the plan.

## 2. Test-drive the change

Follow the red → green → refactor loop ([testing.md](testing.md)) via `/tdd`. Keep the
suite green between commits.

## 3. Review before committing

Run `/rust-review` (or the checklist below) on the diff. Reviewer — human or agent —
checks against the standards, not personal taste.

### Review checklist

- [ ] **Plan honored** — the diff matches the agreed scope; no drive-by changes.
- [ ] **Tests** — a test that fails on `main` covers the change; the right *kind*
      ([testing.md](testing.md)); security tests present for any safety-layer change.
- [ ] **Style** — `fmt` clean, `clippy -D warnings` clean, no unexplained `#[allow]`,
      no `unwrap`/`panic` in library paths ([rust-style.md](rust-style.md)).
- [ ] **Architecture** — code is in the right crate, dependencies point down,
      presentation only in `saya-cli`, files within the size cap
      ([architecture.md](architecture.md)).
- [ ] **Security** — SQL routes through the safety layer, no secrets/`.env`/results
      committed, redaction covers new persisted fields ([security.md](security.md)).
- [ ] **Docs** — new flags/config/behavior documented; `CHANGELOG.md` updated;
      alpha limitations called out honestly.

## Commits & PRs

- **Conventional commits**: `type(scope): summary` — e.g. `feat(agent):`,
  `fix(connectors):`, `test(cli):`, `docs(standards):`, `refactor(store):`,
  `chore(release):`. Scope is usually the crate.
- **No `Co-Authored-By` trailer** on saya commits.
- Keep commits focused; a mechanical refactor and a behavior change are separate
  commits. Explain *why* in the body when it isn't obvious.
- Branch from `main` for work; releases are cut per [`RELEASING.md`](../../RELEASING.md).
- A PR states what changed, why, how it was tested, and any alpha limitation.

## Local gate (must pass before pushing)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked
cargo test --workspace --doc --locked
cargo audit --deny warnings          # when dependencies changed
```

## Decisions that outlive a PR → an ADR

Architectural or cross-cutting decisions (a new crate, a dependency swap, a protocol
change) get a short ADR in `docs/` (see
[`adr-0001-release-architecture.md`](../adr-0001-release-architecture.md)). Record the
context, the decision, and the alternatives rejected — so the next agent doesn't
reopen a settled question.
