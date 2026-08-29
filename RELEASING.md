# Releasing SAYA CLI

SAYA CLI is distributed through several channels, all fed by one tag-triggered
pipeline:

- **GitHub Releases** — prebuilt archives for Linux x86_64, macOS arm64, macOS
  x86_64, and Windows x86_64, each with a SHA-256 sidecar plus a `SHA256SUMS`
  manifest.
- **crates.io** — `cargo install saya-cli` (from source) and
  `cargo binstall saya-cli` (prebuilt binary).
- **Homebrew** — `brew install databook-studio/tap/saya` (tap:
  [databook-studio/homebrew-tap](https://github.com/databook-studio/homebrew-tap)).

## Versioning

- [Semantic Versioning](https://semver.org). While `0.x`, minor bumps may
  contain breaking CLI/config changes; patch bumps are fixes only.
- All workspace crates share one version and are published together. The release
  workflow reads the version from `crates/saya-cli/Cargo.toml`.

## What a tag triggers

Pushing a `vX.Y.Z` tag runs `.github/workflows/release-candidate.yml`, whose jobs
run in order:

1. **build** — a matrix over Linux / macOS-arm64 / macOS-x86_64 / Windows builds
   `saya` in release mode (macOS x86_64 is cross-compiled on the Apple Silicon
   runner — GitHub's Intel runners are scarce), smoke-tests the native binaries,
   and packages an archive + `.sha256`. Tests and Clippy are **not** re-run here;
   the branch ruleset already gates `main` on the full suite.
2. **checksums** — verifies every sidecar and aggregates one `SHA256SUMS`.
3. **publish** — creates the GitHub Release and attaches the archives.
4. **publish-crates** — publishes the workspace to crates.io in dependency order
   via `scripts/publish-crates.sh` (idempotent: skips already-published versions).
5. **bump-homebrew** — regenerates the tap formula from the published
   `SHA256SUMS` via `scripts/update-homebrew-formula.sh` and pushes it to the tap.

Jobs 4 and 5 no-op unless their secrets are configured, so a release never fails
because a channel is not set up.

## Required secrets

| Secret | Used by | What it is |
|---|---|---|
| `CARGO_REGISTRY_TOKEN` | publish-crates | A crates.io API token with publish scope. |
| `HOMEBREW_TAP_TOKEN` | bump-homebrew | A fine-grained PAT with *Contents: read and write* scoped to `databook-studio/homebrew-tap` only. |

Add both under **Settings → Secrets and variables → Actions**. `GITHUB_TOKEN`
cannot push to the separate tap repo, which is why bump-homebrew needs its own
PAT.

## Cutting a release

1. Ensure `main` is green. The branch ruleset requires it: `fmt`, Clippy
   `-D warnings`, and tests on Linux/macOS/Windows, plus the Rust 1.88 MSRV check.
2. Move `CHANGELOG.md` `## Unreleased` to `## X.Y.Z — YYYY-MM-DD` and start a
   fresh `## Unreleased`.
3. Bump the version to `X.Y.Z` in every crate's `Cargo.toml` **and** in the
   internal path-dependency version requirements (the libraries are published
   crates: `saya-store = { path = "...", version = "X.Y.Z" }`). Run
   `cargo check --workspace` to refresh `Cargo.lock`.
4. Open a PR with the version bump + changelog and merge it once green.
   **Squash-merge** anything whose branch history carries internal-only paths —
   CI's `No internal-only paths` job checks the net diff on a PR, which is what
   a squash contributes, not what the intermediate commits held. See
   [Internal-only paths](#internal-only-paths-must-not-reach-a-public-ref).
5. Tag the merged commit and push:
   ```bash
   git tag vX.Y.Z origin/main
   git push origin vX.Y.Z
   ```
   The pipeline then builds, publishes the GitHub Release, publishes to crates.io,
   and bumps the Homebrew formula.

### Validate without publishing

Run **Actions → Release → Run workflow** with `publish: false` to build and
smoke-test the archives on all platforms without creating a release or touching
any channel. (crates.io / Homebrew jobs only run on real `v*` tags.)

### Publish scripts (also runnable locally)

- `scripts/publish-crates.sh` — bottom-up crates.io publish; skips
  already-published versions. Needs `cargo login` locally, or
  `CARGO_REGISTRY_TOKEN` in CI. `DRY_RUN=1` to verify only.
- `scripts/update-homebrew-formula.sh X.Y.Z` — regenerate and push the tap
  formula from the release's `SHA256SUMS`. `DRY_RUN=1` to preview the diff.

### Recovering a partial publish

If publish-crates stops mid-way (for example, a crates.io new-crate rate limit),
use **Re-run failed jobs** on the Actions run — the script is idempotent and
resumes from the first unpublished crate. No re-tag is needed.

## Internal-only paths must not reach a public ref

Agent instructions (`AGENTS.md`, `CLAUDE.md`), editor rules (`.cursor/`),
`.claude/`, and `docs/standards/` are kept out of the distribution. They are
gitignored, but that only helps for files that were never committed.

**Deleting a committed file does not unpublish it.** Once a commit that added
it is reachable from a public ref, `git show <commit>:<path>` returns the
content on an ordinary clone — no special access, no matter that the file is
absent from every current tree and invisible in the GitHub file browser.

`scripts/check-internal-paths.sh` enforces this and CI runs it on every PR and
every push to `main`:

```bash
scripts/check-internal-paths.sh --diff origin/main   # what a squash merge adds
scripts/check-internal-paths.sh origin/main          # what the branch history holds
```

A branch that adds these files and removes them later fails the history form
and passes the diff form. That is not a contradiction — it means **squash-merge
it**. The squashed commit carries the resulting tree, not the intermediate
commits, so nothing enters. This is already the norm here: release branches are
squashed into `main`, which is why only the two files below ever reached it.

### Known exposure, accepted

Two files are reachable from `main` and cannot be recalled without rewriting
published release history:

| Path | Added | Removed |
| --- | --- | --- |
| `.claude/skills/saya-run/SKILL.md` | `4065008` (v0.1.0 initial public release) | `426821b` (0.3.0) |
| `.claude/skills/saya-smoke/SKILL.md` | `4065008` (v0.1.0 initial public release) | `426821b` (0.3.0) |

They shipped in the first public release, so they have been retrievable since
v0.1.0. Both describe how to launch the CLI locally and how to smoke-test the
REPL — no credentials, no infrastructure, nothing not otherwise inferable from
the published docs. Removing them now would mean rewriting `main` and the
release branches, invalidating every clone, fork, and release SHA, and it would
still not reach forks or caches that already hold them. The exposure is
accepted and recorded here rather than repaired.

Fourteen other internal files were also published, on the `feat/memory-data-contracts`
and `release/v0.3.0` branches. Those branches were deleted once their work was
in `main`, which removed the last ref reaching those commits — no history
rewrite was needed, and the tags were unaffected because none of them pointed
into that lineage. A clean clone can no longer retrieve any of the fourteen.

If either grandfathered path is legitimately removed from history later, drop
it from `GRANDFATHERED` in the script so the allowance does not outlive the
reason for it.

## Delete a branch once it is merged

**Automatically delete head branches** is enabled on the repository, so a
merged pull request removes its head branch. Leave it on, and delete by hand
any branch merged outside a PR.

This is not tidiness. A merged branch that stays behind keeps its own commits
reachable, and those commits are not what landed on `main` — a squash merge
puts the *result* on `main` and leaves every intermediate commit alive on the
branch. That is exactly how the fourteen files above stayed public: the work
reached `main` as one squashed commit that contained none of them, while the
branch went on serving all of them to anyone who cloned.

So the two controls cover different halves of the same problem, and neither
substitutes for the other:

| Control | Catches |
| --- | --- |
| `scripts/check-internal-paths.sh` in CI | internal paths entering `main` |
| deleting merged branches | internal paths persisting on a ref that never touched `main` |

The CI check would not have caught the fourteen — they never went near `main`.

### Confirming a branch really is merged

There is no single command for this, and the obvious ones mislead. Work down
this list and stop at the first that answers:

**1. Is it an ancestor of `main`?** Conclusive when true — the branch was merged
with a merge commit and every one of its commits is in `main`.

```bash
git merge-base --is-ancestor origin/<branch> origin/main
```

**2. Does its tip content match `main`?** For a squash merge the commits are not
ancestors, but the content is identical.

```bash
git diff --name-only origin/main origin/<branch>   # empty => tip matches main
```

**3. Did its work flow through another merged branch?** A feature branch is
often an ancestor of the release branch that carried it.

```bash
git merge-base --is-ancestor origin/<branch> origin/<release-branch>
```

**4. Otherwise, read `git log --oneline origin/main..origin/<branch>` and judge.**

Two traps, both of which produce a confident wrong answer:

- **A plain two-dot `git diff main <branch>` also reports `main`'s own advance.**
  A branch that is merely *behind* looks like it has hundreds of unique files.
  `release/v0.2.0` is a proven ancestor of `main` and still shows ~300 differing
  files.
- **Three-dot `git diff main...<branch>` fixes that for a merge commit but not
  for a squash.** It diffs from the merge base, which for a squashed branch sits
  *before* the squash, so all the branch's work reappears as if unmerged.

`release/v0.2.0` gives 306 two-dot and 0 three-dot; `feat/memory-data-contracts`
gave 201 two-dot and 307 three-dot, and was merged. Step 1 or step 3 settles
both; a diff alone settles neither.

GitHub will warn that a squash-merged branch is not merged. That warning is
expected and is not the check — the list above is.

Record the tip SHA before deleting. GitHub keeps unreachable objects for a
while, so `git push origin <sha>:refs/heads/<branch>` restores it within that
window.

## Security

- Never commit secrets. `.env*`, `.saya/config.toml`, `.saya/connections.toml`,
  and session files are gitignored; API keys and DB passwords are supplied via
  `SecretRef` references (`{ env = ... }` / `{ file = ... }`).
- Internal-only paths are a separate concern from secrets; see the section
  above. Both share the property that a later deletion does not undo the
  publication.
- crates.io publishes are permanent (versions can be yanked but not deleted or
  overwritten); double-check the version before tagging.
- Report vulnerabilities per [`SECURITY.md`](SECURITY.md).
