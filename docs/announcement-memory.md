# SAYA remembers what you tell it about your database

*Feature announcement — memory and data contracts*

Every database has facts that aren't in its schema.

`orders` has both `created_at` and `ordered_at`. Only one is the reporting time column. The catalog
can't tell you which; the person who built the pipeline can. "Customers" means the `accounts` table,
not the `customers` view nobody has used since the migration. `tier_code = 3` means the account is in
collections.

You explain this to a colleague once. You explain it to an AI assistant every single time.

SAYA now remembers. Tell it once:

```bash
saya contracts remember analytics.public.orders --kind time-column --value created_at
```

and later questions carry that context. Change your mind and it reverts on the next question. That's
the whole feature — and almost all of the work went into what it *won't* do.

---

## It refuses to guess

The easy version of this feature watches you work, infers what your tables mean, and quietly gets
better. That version is also the one that confidently tells you last quarter's revenue using a column
that was renamed in March.

So the design is mostly restraint:

**It never confirms anything by itself.** The model can *propose* facts — but a proposal is inert
until a human confirms it, no matter how many times it has been observed. Repetition is not evidence
of meaning. A query running successfully proves it executed, not that anyone understood it.

**It won't pick between contradictions.** Confirm two different grains for the same table and it
shows you both, marks them disputed, and tells the model not to choose. It doesn't prefer the newer
one or the better-supported one. Resolving the disagreement is your call, and quietly picking would
be making it for you without saying so.

**It won't accept a fact about a table that doesn't exist.** Misspell the name and it tells you
immediately, naming the command that would fix it — rather than accepting it and marking it broken
later, when you've forgotten you typed it.

**It won't invalidate your knowledge over a network blip.** If a database is unreachable, nothing is
marked stale. An unreachable database isn't evidence that anything changed, and marking claims stale
because a VPN dropped would destroy work you spent months accumulating.

**And it's off by default.** Memory reads nothing and writes nothing until you turn it on.
Upgrading changes nothing until you decide otherwise.

---

## It notices when the world moves

A remembered fact records the shape of the table it describes — the columns it depends on, their
types, whether they're nullable.

Rename the column a fact depends on and the next schema refresh marks it stale. It leaves what the
model sees immediately and appears in a review queue instead, where you can repair it or drop it.
An unrelated column added elsewhere doesn't invalidate anything.

The distinctions are deliberate, because "the schema changed" isn't one event:

| What changed | What happens |
| --- | --- |
| a column the fact depends on was removed or renamed | **stale** — it stops being used |
| that column changed type | **stale** |
| that column became nullable | **stale** — nulls change what a fact means |
| that column stopped being nullable | flagged for review — narrower, not wrong |
| an unrelated column appeared | flagged for review, still used |
| the database was unreachable | nothing marked |

A "default time column" that is now sometimes null breaks exactly the queries the fact exists to
shape. A column that merely stopped accepting nulls narrowed something already true. Same schema
diff, different severity.

---

## It shows you what it assumed

Every turn names what memory put in front of the model, before the model is called:

```
memory supplied · 2 claims (1 unconfirmed)
  pagila.public.rental  [current]  (profile: docker_postgres)
    ki-a86a…  default_time_column  return_date  active
    ki-4f21…  table_grain  one row per rental  pending  (unconfirmed)
```

It says **supplied**, not *used*. Recall puts a fact in context; whether the SQL honoured it is a
different question, and one SAYA doesn't measure. Saying "applied" would claim more than it knows.

And when the SQL contradicts something you established, it says so:

```
memory overridden · 1 finding
  ki-a86a…  referenced rental_date where you specified return_date
```

That notice exists because the honest version of this feature admits its own limit: a remembered
fact is strong context, not an instruction the model must obey.

---

## What it stores, and what it can't

Facts live in a private SQLite database on your machine, `0600` in a `0700` directory. A stored fact
is one short typed statement — a description, an alias, a grain, a column's role, a time column —
bound to a fully qualified table and an opaque one-way identity for the connection.

It cannot hold your SQL, result rows, prompts, credentials or file paths. Not "is filtered of" —
*cannot hold*: the types have no variant capable of it, and content shaped like a credential or a
statement is refused at storage rather than scrubbed, because storing a laundered version would hide
that something produced it.

Two classes can't be caught by inspection, and the documentation says so rather than implying
otherwise: an opaque token and a single copied cell look exactly like a legitimate product code. Those
are kept out by never putting result rows in front of the extractor in the first place.

**Recalled facts reach the model as quoted, delimited data marked untrusted and possibly stale — never
as instruction.** A fact containing text that looks like a command stays inside its wrapper. It cannot
enable a tool, change an approval mode, or widen what SAYA is willing to run. Every generated
statement still passes the same read-only safety layer it always did, and a remembered fact can never
introduce a table or column that doesn't exist.

**Privacy is decided per request.** With a cloud provider and data sharing disabled, no contract
content is sent — the store isn't queried at all, and the memory tools aren't offered to the model.
Change provider or setting and it takes effect on your next question.

---

## Deleting means deleting — with one honest caveat

`saya contracts forget <id>` stops a fact influencing anything immediately, and erases its content and
its evidence in the same transaction that records the deletion.

The row survives, carrying no content: an id, a table, a status, timestamps. That's deliberate, and
it buys two things — "why did SAYA stop using that?" stays answerable, and remembering the same fact
again reports **previously forgotten** rather than silently resurrecting something you deleted.

If you want the row gone rather than emptied, that isn't implemented yet. Deleting the state database
removes everything.

---

## Getting started

```toml
# .saya/config.toml — memory is off until you turn it on
[memory]
mode = "assisted"        # off | assisted
```

```bash
saya contracts remember <table> --kind <kind> --value <value>
saya contracts list
saya contracts show <table>
saya contracts queue          # what's waiting for a decision
saya contracts forget <id>
```

The same operations are `/contracts [table]`, `/remember`, `/forget` and `/queue` in the REPL and
TUI — the same code underneath, so they can't disagree with each other.

Full behaviour, including what's stored and what `forget` does and doesn't erase:
[docs/memory.md](memory.md).
