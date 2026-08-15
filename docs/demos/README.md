# Memory demo recordings

Five short scenarios, each recordable with [VHS](https://github.com/charmbracelet/vhs) from the
repository root:

```bash
cargo build --release
vhs docs/demos/memory-remember.tape
```

Each tape calls `_fixture.sh` first, which builds a throwaway SQLite database with the same rows and
the same schema every time — so a re-recording differs only where the product changed.

**Before recording**, replace `<path>` in each tape with the path to the built binary
(`target/release/saya`). It is a placeholder rather than a hardcoded path so the tapes do not silently
record a stale binary.

| Tape | Scenario | The point |
| --- | --- | --- |
| `memory-remember` | remember a time column, list it | tell it once, it stops re-deriving |
| `memory-refuses` | a table that does not exist; two contradictory grains | it refuses to guess |
| `memory-drift` | rename a column, refresh, watch the claim leave recall and enter the queue | it notices when the schema moves |
| `memory-forget` | forget, then remember the same fact again | forgetting means forgotten, and it says so |
| `memory-team` | export, commit, a teammate imports | shared knowledge in version control |

## What each scenario actually shows

Every one of these was driven against a real database before being scripted. The outputs in the tapes
are what the binary printed, not what the feature is supposed to print.

**`memory-remember`** — the honest framing is that saya already knows the *schema*; what it does not
know is what the columns **mean**. `created_at` versus `ordered_at` is not a question the catalog can
answer.

**`memory-refuses`** — two refusals in one recording. A misspelled table is rejected at the moment
you make the mistake, naming the command that would fix it, rather than being accepted and quietly
marked stale later. And two contradictory grains are both kept and marked disputed: the model is told
they disagree and instructed not to pick, because picking would be a decision made on your behalf
without telling you.

**`memory-drift`** — the claim disappears from what the model sees *and* appears in the review queue,
in the same step. Those are two different audiences for the same fact.

**`memory-forget`** — the second `remember` reporting *previously forgotten* is the part worth
watching. A deleted fact that silently comes back would make deletion meaningless.

**`memory-team`** — the exported file contains the qualified object name and nothing identifying the
machine that wrote it: no profile identity, no evidence, no session ids, no absolute paths. The claim
ids are identical on the teammate's machine, because identity is derived rather than assigned.

## What these deliberately do not show

- **Learning.** `learning` defaults to `off` and these recordings leave it there. A demo of the model
  proposing facts would need a live provider, which makes the recording non-deterministic.
- **A query changing.** Showing recall alter generated SQL needs a real model call. It is the most
  compelling scenario and the least reproducible; `docs/demo-live.tape` is the existing pattern for
  recordings that need a gateway.
