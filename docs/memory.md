# Memory

SAYA can remember what you tell it about your databases — that `orders.created_at` is the reporting
time column, that "customers" means `analytics.public.accounts`, that a table has one row per shipped
order — and use that when building later queries.

**Memory is off by default.** Turning it on is one setting. Nothing accumulates until you do.

## What SAYA remembers

A **knowledge item** is one short, typed statement about one fully qualified database object, bound
to the connection profile it was made under.

Each item occupies a **slot** — the position it fills for that object:

| Slot | Example | How many |
| --- | --- | --- |
| `table.description` | one row per shipped order | several |
| `table.alias` | customers | several |
| `table.grain` | one row per customer per month | **one** |
| `table.default_time` | `created_at` | **one** |
| `column:<name>.description` | the account's billing tier | several |
| `column:<name>.role` | `identifier`, `dimension`, `measure`, `timestamp`, `sensitive` | **one** |
| `relation.join_rule` | `orders.customer_id = customers.id, and only where customers.is_active` | several |
| `metric.definition` | `mrr = SUM(subscription_amount) WHERE status = 'active'` | several |

Single-valued slots hold exactly one value. Telling SAYA a new grain for a table replaces the old
one; it does not accumulate a second opinion for something to arbitrate later. The database enforces
this, not a convention in the code.

An item is **not** free text and not a note. It cannot contain SQL, credentials, file paths, or
instructions — those shapes are refused at storage rather than filtered, and the payload types have
no variant capable of holding them.

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
saya contracts forget ki-1a2b3c…
```

In the REPL and TUI the same operations are `/contracts [table]`, `/remember`, `/forget`,
`/queue`, `/confirm` and `/reject`. `/contracts` lists every contract; `/contracts
analytics.public.orders` shows one. They call the same code as the commands above, so they cannot
disagree.

## Configuration

```toml
[memory]
mode = "off"                   # off | assisted
max_contracts = 5
max_claims_per_contract = 12
max_context_bytes = 16384
```

**`off`** — nothing is read and nothing is written. No store access at all.

**`assisted`** — what you state explicitly becomes active knowledge and is supplied to the model on
later turns. What SAYA infers from a conversation becomes a pending proposal, labelled unconfirmed
wherever it appears, until you keep or dismiss it.

The three numbers bound what reaches the model on any one turn: at most five objects, twelve items
per object, and 16 KiB in total. Truncation is explicit — when items are dropped for space, the turn
says so rather than presenting a subset as the whole.

## What every turn tells you

Before the model is called, SAYA prints what memory supplied:

```
memory supplied · 2 claims (1 unconfirmed)
  pagila.public.rental  [current]  (profile: docker_postgres)
    ki-a86a…  default_time_column  return_date  active
    ki-4f21…  table_grain  one row per rental  pending  (unconfirmed)
```

It says **supplied**, not *used*. Recall places items in the model's context; whether the generated
SQL honours them is a separate question, and one SAYA does not currently measure. Anything stronger
would overstate what is known.

The short id (`ki-a86a…`) is the reference you type:

```
/confirm ki-a86a      keep a pending item — it becomes active
/reject  ki-4f21      dismiss it
```

An ambiguous or unrecognised prefix is refused and changes nothing. Ids are immutable, so a prefix
you read a minute ago still names the same item.

## When SAYA disagrees with you

If the SQL SAYA generated contradicts an active item, the turn says so:

```
memory overridden · 1 finding
  ki-a86a…  referenced rental_date where you specified return_date
```

It says **referenced**, not *used as the time column*. The check reads the statement's object and
column names; it cannot tell a filter from a projection, so it reports what it can prove.

This reports; it does not block. The answer still returns. **An active item does not compel the
model** — it is strong context, not an enforcement mechanism, and the override notice exists because
that distinction is real.

## Learning

Under `assisted`, SAYA extracts proposals after a turn from a bounded record of what happened: what
you asserted, which objects the tools inspected, which the SQL touched, and the answer.

The extractor is offered only the objects the turn actually involved, identified by turn-scoped
references rather than free-text names, so it cannot propose a fact about a table that was never
there. SAYA — not the model — assigns the profile, the qualified object, the schema binding, and the
initial state.

A proposal duplicating something already supplied that turn is dropped. Otherwise a fact could
strengthen itself simply by being recalled.

Extraction is best-effort and isolated: if it fails, times out, or returns nothing usable, your query
and your answer are unaffected.

It is not silent about it, though. A turn whose extraction failed or timed out prints
`memory not recorded · …` beneath the answer, so a fact you stated and expected to stick is never
quietly dropped. A turn the gate declined — most ordinary turns, where there was nothing durable to
learn — says nothing, because a line on every turn would train you to ignore the line that matters.

If you want to see the boundary itself, `--verbose` (or `SAYA_EXTRACTION_TRACE=1`) reports the gate
decision, how many objects the turn involved, the outcome, and how many facts were recorded. It is
off by default and never prints the model's raw response, which can carry your data.

## What SAYA stores, and what it never stores

Stored: the object's qualified name, the profile identity, the slot, a typed value, who said it
(you, or SAYA's inference), its state, and the schema dependency the fact rests on.

Never stored: SQL text, result rows, credentials, connection strings, file paths, or free-form
prose. Structurally recognisable secrets — PEM blocks, credential headers, absolute paths, URLs with
inline credentials — are refused at admission, and a byte scan of the database file and its
write-ahead sidecars asserts they never reach disk.

**What that cannot catch:** a bare token like `hunter2` has no structure distinguishing it from a
legitimate business term like `SHIPPED`. A check that rejected one would reject both. Do not put a
secret in a description and expect it to be caught.

## Privacy

Memory is local. Items are stored in SAYA's own SQLite database with `0600` permissions inside a
`0700` directory.

When `allow_data_sharing = false`, no database-derived knowledge reaches a cloud provider,
regardless of memory mode. The privacy gate is independent of the memory setting and wins over it —
a mode is a preference, sharing is a boundary.

## Deletion

`saya contracts forget <id>` stops an item influencing anything immediately: it leaves recall,
listings, and the model's view on the next turn.

Its content is erased. The value and the schema dependency are blanked in the same transaction that
records the deletion, the freed space is overwritten rather than left in the page, and the
write-ahead log is folded back so the original text does not survive in a sidecar. A byte scan
asserts this — an API that returns nothing while the text remains readable on disk is not deletion.

**The row itself remains**, carrying no content: its id, its object, its slot, its state and its
timestamps. That keeps "why did SAYA stop using that?" answerable, and means re-remembering the same
fact reports it was *previously forgotten* rather than silently resurrecting it.

If you need the row gone rather than emptied, that is not implemented. Deleting the state database
removes everything.

## When your schema changes

An item depends only on what it actually uses:

| Slot | Depends on |
| --- | --- |
| description, alias, grain | the object existing |
| default time column | that column existing, and still being a time column |
| column description, column role | that column existing |
| join rule | the local join keys existing (the table, when the rule has no keys) |
| metric definition | the underlying columns existing (the table, when the metric names none) |

So a colleague adding an unrelated column does **not** disturb a fact that never referred to it. That
matters more than it sounds: a memory feature that flags everything after every migration gets
ignored, and then it is worth nothing.

If the column a fact names disappears, or stops being the kind of column the fact assumed, the item
is invalid and stops reaching the model. It stays visible in `contracts list` and `contracts show` so
you can see what changed, and in `contracts queue` to keep, correct or forget.

**If the database cannot be reached, nothing is marked.** An unreadable schema is not evidence a
column is gone, and treating it as such would destroy knowledge over a network blip.

## Contradictions

Two conflicting values for a single-valued slot cannot both exist — the newer replaces the older when
you state it. There is no queue of disagreements to arbitrate, because the shape of the data prevents
the disagreement from being stored.

For multi-valued slots, several descriptions or aliases coexist by design; they are alternatives, not
rivals.

## If something goes wrong

`saya contracts list` shows every item SAYA holds for the active profile with its state and whether
its dependency still holds. `saya contracts queue` shows what is pending review.

If the memory store is unavailable, queries and answers still work — memory degrades answer quality,
never availability. The turn says the store could not be read rather than pretending memory was
empty.
