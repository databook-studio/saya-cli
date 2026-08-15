# Memory and data contracts

SAYA can remember what you tell it about your databases — that `orders.created_at` is the reporting
time column, that "customers" means `analytics.public.accounts`, that a table has one row per shipped
order — and use that context when building later queries.

**Nothing is remembered unless you ask.** Recall of confirmed facts is on by default; learning is
off. Upgrading SAYA changes nothing until you change a setting.

## What a claim is

A **claim** is one short, typed statement about one fully qualified database object, bound to the
connection profile it was made under and to the shape that object had at the time.

| Kind | Example |
| --- | --- |
| `description` | one row per shipped order |
| `alias` | customers |
| `grain` | one row per customer per month |
| `column-description` | the account's billing tier |
| `column-role` | `identifier`, `dimension`, `measure`, `timestamp`, `sensitive` |
| `time-column` | `created_at` |

A claim is **not** free text, and it is not a note. It cannot contain SQL, credentials, file paths,
or instructions — those shapes are refused at storage rather than filtered, and the payload types
have no variant capable of holding them.

## Getting started

```bash
# Tell SAYA something
saya contracts remember analytics.public.orders --kind time-column --value created_at

# See what it knows
saya contracts list
saya contracts show analytics.public.orders

# Ask a question that benefits from it
saya ask "orders by month"

# Change your mind
saya contracts forget c-1a2b3c…
```

In the REPL and TUI the same operations are `/contracts`, `/contract <table>`, `/remember`,
`/forget` and `/queue`. They call the same code as the commands above, so they cannot disagree.

## Configuration

```toml
[memory]
recall = "confirmed"           # off | confirmed | include-candidates
learning = "off"               # off | suggest | auto-candidate
max_contracts = 5
max_claims_per_contract = 12
max_context_bytes = 16384
retention_days = 180
```

**`recall`** — what reaches the model when you ask a question.

- `off` — nothing. The store is not even queried.
- `confirmed` *(default)* — facts you or your team confirmed.
- `include-candidates` — also unreviewed suggestions, each marked `[candidate — unconfirmed]` in the
  context so the model cannot read one as established.

**`learning`** — whether SAYA proposes new claims.

- `off` *(default)* — it never writes anything on its own.
- `suggest` — after a turn it reports what it *would* have proposed, and stores nothing.
- `auto-candidate` — it may store **candidates**, which are inert until you confirm them.

No setting makes SAYA confirm a claim on its own. Confirmation is a human action, with one
exception: a claim imported from a reviewed team file (see below).

## Reviewing what it proposes

```bash
saya contracts queue                 # candidates and stale claims awaiting a decision
saya contracts review c-1a2b… --confirm
saya contracts review c-1a2b… --reject
```

The queue orders by evidence, then age, then id — stable between runs, so you can work through it.
Each entry shows the claim, the object, the schema state and how many observations support it.

Only confirmed claims influence query building. A candidate never does, no matter how often it has
been observed: repetition is not confirmation.

## What SAYA stores, and what it never stores

Stored, in a private SQLite database under your state directory (`0600`, in a `0700` directory):

- the claim's typed value, its origin, its status and timestamps
- the fully qualified object and an opaque, one-way profile identity
- a digest of the object's shape, plus the name, type and nullability of the columns the claim
  depends on
- bounded evidence references: which session and turn observed something, and when

**Never stored**: your SQL, result rows or cell values, prompts, provider payloads, credentials,
connection strings, or absolute file paths. Tests scan the database file and its write-ahead log for
planted examples of each after a full lifecycle.

Two classes cannot be detected by inspection and are kept out by construction instead: an opaque
token and a single copied result cell look exactly like a legitimate product code or status name.
Nothing puts result rows in front of the extractor, and the payload types cannot hold arbitrary text.

## Privacy

Contract content is **database-derived** and follows your existing data-sharing setting. With a cloud
provider and sharing disabled, no contract content is sent — the store is not queried at all, and the
contract tools are not offered to the model. Local providers follow the usual local-provider rules.

This is re-decided on every request, so changing provider or privacy setting takes effect on the next
question rather than the next session.

## Deletion — read this carefully

`saya contracts forget <id>` stops a claim influencing anything immediately: it disappears from
recall, from listings, and from the model's view on the next turn. Its content is erased — the
payload and the referenced columns are cleared, and its evidence rows are deleted — in the same
transaction that records the deletion.

**The row itself remains, carrying no content**: its id, its object, its status and its timestamps.
This is deliberate. It keeps "why did SAYA stop using that?" answerable, and it means re-proposing
the same fact reports *previously forgotten* rather than silently resurrecting it.

If you need the row gone rather than emptied, that is not yet implemented. Deleting the state
database removes everything.

## When your schema changes

A claim records the shape of the object it describes. On an explicit schema refresh, SAYA compares
that against the live database:

| What changed | What happens |
| --- | --- |
| nothing | the claim stays current |
| an unrelated column elsewhere | the claim is flagged for review, not invalidated |
| a column the claim depends on was removed or renamed | the claim is marked **stale** |
| a column it depends on changed type | marked **stale** |
| a column it depends on became nullable | marked **stale** — nulls change what a claim means |
| a column it depends on stopped being nullable | flagged for review |
| the database could not be reached | **nothing is marked** |

That last row matters: an unreachable database is not evidence that anything changed, and marking
claims stale over a network blip would destroy knowledge you spent time building.

Stale claims stop reaching the model and appear in `contracts queue` so you can confirm, edit or
forget them.

## Contradictions

If two confirmed claims disagree — two different grains for one table — SAYA shows both, marks them
`[disputed]`, and tells the model not to choose between them silently. It does not rank them, prefer
the newer one, or hide one. Resolving the disagreement is your decision; `contracts show` displays it
and `review` acts on it.

## Team contracts

Contracts can be committed to a repository as TOML under `.saya/contracts/`:

```toml
version = 1
object = "analytics.public.orders"

[[claims]]
kind = "description"
value = "one row per shipped order"

[[claims]]
kind = "time-column"
value = "created_at"
```

```bash
saya contracts import --dry-run     # report what would change
saya contracts import               # apply it
saya contracts export ./contracts   # write your confirmed claims out
```

Imported claims are **confirmed**, because a file in version control has already been reviewed by
whoever merged it. That makes `.saya/contracts/` a trusted input: treat write access to it the way
you treat write access to the code.

Discovery is bounded — 32 files, 64 KiB each, 1 MiB total — and refuses anything that is not a
regular `.toml` file inside the directory. A symlink pointing outside it is rejected rather than
followed. Unknown fields are errors, so a contract cannot appear to be in force while being ignored.

Exported files carry the qualified object name and nothing identifying your machine: no profile
identity, no evidence, no session ids, no absolute paths. Relationship claims are skipped, and the
count is reported, because the file format has no word for them yet.

## Preferences

Separate from claims, because they describe you rather than a database object:

```bash
saya preferences set timezone Europe/London
saya preferences set output-style compact
saya preferences list
```

`timezone` and `date-grain` are per profile; `output-style` and `default-profile` are global. A
preference cannot contain SQL, secrets or instructions — the type has no variant that could.

Nothing consumes preferences yet; they are stored and shown.

## If something goes wrong

**Memory never breaks the ordinary path.** If the store is unavailable, questions and queries work
exactly as they would without it, with a diagnostic. A remembered claim can never introduce a table
or column that does not exist, and can never authorise SQL: every statement still passes the same
read-only safety layer.

Recalled context reaches the model as quoted, delimited data marked untrusted and possibly stale —
never as instruction. A claim containing text that looks like an instruction stays inside its
wrapper; it cannot enable a tool, change an approval mode, or alter what SAYA is willing to run.
