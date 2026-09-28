# A recorded, reproducible walkthrough

One synthetic journey, end to end: build the demo database, watch `saya
setup` refuse to run from a script, ask the demo database a question that
trips one of its planted SQL traps, save that exact SQL as a portable
investigation, run it, export it, import it into a **second** isolated
environment ("bob"), run it there, and watch a stale review get refused —
with **no AI provider and no network** anywhere in the run.

- Script: [`scripts/walkthrough.sh`](../scripts/walkthrough.sh) — the CLI journey, recorded below
- Recording: [`walkthrough.gif`](walkthrough.gif) — the same journey as slash commands inside the TUI, rendered from [`walkthrough.tape`](walkthrough.tape)

## Reproduce it

```bash
bash scripts/walkthrough.sh                      # uses `cargo run -q -p saya-cli --`
SAYA_BIN=/path/to/saya bash scripts/walkthrough.sh
WALKTHROUGH_KEEP=1 bash scripts/walkthrough.sh   # keep the sandbox for inspection
```

The script prints each command before its output, checks every exit code
against the expected one, and exits non-zero on any unexpected result
(including the deliberate refusals below, whose exit codes are asserted,
not merely tolerated). Everything runs against **synthetic data only**:
the demo database is a local SQLite file of 240 fake customers and 560
fake orders, and every command takes the bounded, read-only path. Two
isolated roots, `alice/` and `bob/`, each get their own `HOME`,
`SAYA_CONFIG_HOME`, `SAYA_STATE_DB`, `SAYA_DEMO_DIR`, and
`SAYA_INVESTIGATIONS_DIR`, so the export/import leg really crosses between
two environments — nothing but the exported JSON file travels between
them, and the sandbox is deleted at the end unless `WALKTHROUGH_KEEP=1`.

One step of the journey is a *refusal by design*: `saya setup` is a guided,
interactive flow that needs a terminal. From a script it refuses and points
at `saya config init` or `saya demo` instead — the walkthrough shows that
refusal and asserts its exit code, which is what a scripted environment
should see.

## The recorded journey

This is the recorded output of `bash scripts/walkthrough.sh` on this tree,
with only the sandbox paths, timestamps, and volatile id/exec suffixes
trimmed to `…` (the script captures the investigation id from `save`'s
output rather than hardcoding it; the hash suffix after the slug changes
from run to run):

```text
== 0 · Environment ==
Binary: cargo run -q -p saya-cli -- (override with SAYA_BIN=…)
Sandbox: … — two isolated roots, alice/ and bob/
Isolated per root: HOME, SAYA_CONFIG_HOME, SAYA_STATE_DB, SAYA_DEMO_DIR, SAYA_INVESTIGATIONS_DIR
No AI provider is configured and nothing here touches the network.
== 1 · Alice builds the demo database (synthetic customers and orders) ==
$ cargo run -q -p saya-cli -- demo --non-interactive
Demo database (built a new fixture): …/alice/demo/demo.sqlite3
Connections file: …/alice/demo/connections.toml
Open it read-only:
  saya --connections …/alice/demo/connections.toml --profile demo
Example SQL:
  SELECT count(*) FROM customers;
  SELECT count(*) FROM orders WHERE order_date >= '2025-12-31';
  SELECT c.id, count(*) AS contact_rows FROM customers c JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.id ORDER BY contact_rows DESC LIMIT 5;
== 2 · `saya setup` needs a terminal — from a script it refuses and points elsewhere ==
(the guided `saya setup` itself is interactive; this walkthrough only shows the scripted refusal)
$ cargo run -q -p saya-cli -- setup --non-interactive
saya setup is interactive. For a scripted start use `saya config init` (templates) or `saya demo` (sample database).
== 3 · Alice asks the demo database — the honest counts first ==
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive query --sql 'SELECT count(*) AS customers FROM customers'
customers
240
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive query --sql 'SELECT count(*) AS contact_rows, count(DISTINCT cc.customer_id) AS customers_with_contacts FROM customer_contacts cc'
contact_rows	customers_with_contacts
123	93
== 4 · The trap: join customer_contacts and the numbers stop agreeing ==
joined_rows counts rows; distinct_orders counts real orders. The join repeats an order once per contact row.
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive query --sql 'SELECT c.region, count(*) AS joined_rows, count(DISTINCT o.id) AS distinct_orders FROM orders o JOIN customers c ON c.id = o.customer_id JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.region ORDER BY joined_rows DESC'
region	joined_rows	distinct_orders
north	83	59
west	76	60
south	73	60
east	69	47
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive query --sql 'SELECT c.region, count(*) AS orders FROM orders o JOIN customers c ON c.id = o.customer_id GROUP BY c.region ORDER BY orders DESC'
region	orders
north	155
west	142
south	142
east	121
== 5 · Alice saves the trap query as a portable investigation ==
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive investigation save --name 'Orders-per-region via contact join' --description 'Duplicate-join trap: order rows multiplied by customer_contacts' --sql 'SELECT c.region, count(*) AS joined_rows, count(DISTINCT o.id) AS distinct_orders FROM orders o JOIN customers c ON c.id = o.customer_id JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.region ORDER BY joined_rows DESC'
orders-per-region-via-contact-join-…
{
  "format": "saya.investigation",
  "version": 1,
  "id": "orders-per-region-via-contact-join-…",
  "revision": 1,
  "name": "Orders-per-region via contact join",
  "description": "Duplicate-join trap: order rows multiplied by customer_contacts",
  "sql": "SELECT c.region, count(*) AS joined_rows, count(DISTINCT o.id) AS distinct_orders FROM orders o JOIN customers c ON c.id = o.customer_id JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.region ORDER BY joined_rows DESC",
  "dialect": "sqlite",
  "connection": "demo",
  "objects": [
    "orders",
    "customers",
    "customer_contacts"
  ],
  "schema_fingerprint": null,
  "created_unix_ms": …,
  "updated_unix_ms": …
}
Saved exactly as shown. Review the SQL before sharing: literals are stored verbatim.
== 6 · list and show record exactly what was saved ==
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive investigation list
orders-per-region-via-contact-join-…  1  sqlite  demo  Orders-per-region via contact join
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive investigation show orders-per-region-via-contact-join-…
{
  "format": "saya.investigation",
  "version": 1,
  "id": "orders-per-region-via-contact-join-…",
  "revision": 1,
  "name": "Orders-per-region via contact join",
  "description": "Duplicate-join trap: order rows multiplied by customer_contacts",
  "sql": "SELECT c.region, count(*) AS joined_rows, count(DISTINCT o.id) AS distinct_orders FROM orders o JOIN customers c ON c.id = o.customer_id JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.region ORDER BY joined_rows DESC",
  "dialect": "sqlite",
  "connection": "demo",
  "objects": [
    "orders",
    "customers",
    "customer_contacts"
  ],
  "schema_fingerprint": null,
  "created_unix_ms": …,
  "updated_unix_ms": …
}
local binding: demo (reviewed revision 1)
== 7 · Alice runs the saved investigation — same query, same evidence line ==
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive investigation run orders-per-region-via-contact-join-…
region	joined_rows	distinct_orders
north	83	59
west	76	60
south	73	60
east	69	47
saved investigation: demo · 4 rows · exec … · full result
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive investigation run orders-per-region-via-contact-join-… --report …/shared/report.md
region	joined_rows	distinct_orders
north	83	59
west	76	60
south	73	60
east	69	47
saved investigation: demo · 4 rows · exec … · full result
Wrote report to …/shared/report.md (rows omitted)
== 8 · Alice exports the portable definition (no binding, no rows, no credentials) ==
$ cargo run -q -p saya-cli -- --connections …/alice/demo/connections.toml --profile demo --non-interactive investigation export orders-per-region-via-contact-join-… …/shared/investigation.json
Exported orders-per-region-via-contact-join-… to …/shared/investigation.json.
The file holds the portable definition only — review the SQL before sharing.
== 9 · Bob builds the same demo database (the fixture is deterministic) ==
$ cargo run -q -p saya-cli -- demo --non-interactive
Demo database (built a new fixture): …/bob/demo/demo.sqlite3
Connections file: …/bob/demo/connections.toml
Open it read-only:
  saya --connections …/bob/demo/connections.toml --profile demo
Example SQL:
  SELECT count(*) FROM customers;
  SELECT count(*) FROM orders WHERE order_date >= '2025-12-31';
  SELECT c.id, count(*) AS contact_rows FROM customers c JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.id ORDER BY contact_rows DESC LIMIT 5;
== 10 · Bob imports the definition — a preview, and nothing executes ==
$ cargo run -q -p saya-cli -- --connections …/bob/demo/connections.toml --non-interactive investigation import …/shared/investigation.json
id: orders-per-region-via-contact-join-…
name: Orders-per-region via contact join
dialect: sqlite
connection: requires --connection <profile> to run (saved alias "demo")
objects: orders, customers, customer_contacts
sql:
SELECT c.region, count(*) AS joined_rows, count(DISTINCT o.id) AS distinct_orders FROM orders o JOIN customers c ON c.id = o.customer_id JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.region ORDER BY joined_rows DESC
Imported without a local connection. Run with --connection <profile> to map it; nothing was executed.
== 11 · Bob's first run is refused — no connection mapping travelled with the file ==
$ cargo run -q -p saya-cli -- --connections …/bob/demo/connections.toml --non-interactive investigation run orders-per-region-via-contact-join-…
no local connection mapped: pass --connection <profile>
== 12 · Bob runs it on his own demo database with an explicit --connection ==
$ cargo run -q -p saya-cli -- --connections …/bob/demo/connections.toml --non-interactive investigation run orders-per-region-via-contact-join-… --connection demo
region	joined_rows	distinct_orders
north	83	59
west	76	60
south	73	60
east	69	47
saved investigation: demo · 4 rows · exec … · full result
== 13 · Bonus: a stale review is refused without any ALTER ==
A second SQLite database with the same table names but a different shape
(orders gains a note column; customer_contacts loses channel). No ALTER ran —
the differently-shaped schema was built directly, and the review refuses it.
skewed schema:
CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL, order_date TEXT NOT NULL, amount_cents INTEGER, status TEXT NOT NULL, note TEXT);
CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, region TEXT NOT NULL);
CREATE TABLE customer_contacts (customer_id INTEGER NOT NULL, value TEXT NOT NULL);
$ cargo run -q -p saya-cli -- --connections …/bob/skewed-connections.toml --non-interactive investigation run orders-per-region-via-contact-join-… --connection skewed
review is stale (target changed, schema changed); pass --revalidate to re-review the current state
== Walkthrough complete ==
All commands behaved as recorded; exit 0.
```

The recorded run exited `0`.

## What just happened

**Steps 1–2 — the sandbox and the refusal.** `saya demo --non-interactive`
builds a deterministic SQLite fixture and a `connections.toml` whose single
profile (`demo`) opens it read-only. `saya setup --non-interactive` refuses:
the guided flow needs a terminal, and a script is pointed at `saya config
init` or `saya demo` instead. Exit code 2 is asserted, not tolerated.

**Step 3 — the honest counts.** 240 customers; 123 contact rows over 93
customers. Keep those two numbers apart — that gap is the trap.

**Step 4 — the duplicate-join over-count.** The demo plants
`customer_contacts` (1–3 rows per customer) precisely so that joining
through it multiplies rows. The naive region report joins `orders →
customers → customer_contacts`: `count(*)` reports 301 rows where only 226
distinct orders exist among contact-having customers — each order is
repeated once per contact row. Worse, the inner join silently drops every
order of a contactless customer: the true per-region counts are north 155,
west 142, south 142, east 121 (560 in total). `count(DISTINCT o.id)` is
what exposes the multiplication; dropping the join is what fixes it.

**Steps 5–8 — save, list, show, run, report, export.** `investigation
save` stores one portable JSON document: the exact SQL (verbatim — review
it before sharing), its name and description, the dialect, the connection
alias, and the referenced tables. No credentials, no rows, no machine
identity. The per-machine review binding (`local binding: demo (reviewed
revision 1)`) stays local and is never exported. `investigation run`
replays through the same bounded read-only query path as `saya query` and
prints the evidence line (`saved investigation: demo · 4 rows · exec … ·
full result`); `--report` writes the same facts into a shareable Markdown
file — exact SQL and provenance by default, rows omitted unless asked.
`investigation export` writes only the definition to a file for sharing.

**Steps 9–12 — bob's side.** Bob builds the same deterministic fixture, so
his database has identical contents. `investigation import` validates the
whole document, previews it, and stores it with **no** connection mapping;
nothing executes. His first `investigation run` is therefore refused — the
connection mapping is per-machine state that never travels with the file.
With `--connection demo` he reviews and runs it on his own database, and the
same numbers come back: the fixture is deterministic, the review binds on
success.

**Step 13 — stale review without any ALTER.** A second database with the
same table names but a different shape (an extra `note` column on `orders`,
no `channel` on `customer_contacts`) is *built* directly — no `ALTER` ever
runs. The replay against it is refused before anything executes: the review
covers the definition, the target, and the referenced objects' schema, and
the recorded binding no longer matches (`--revalidate` is the documented
way to re-review, which this walkthrough deliberately does not take).

## The same journey in the TUI

`bash scripts/walkthrough.sh` drives the headless CLI. Inside `saya` (the
interactive session) the same operations exist as slash commands; the
recording [`walkthrough.gif`](walkthrough.gif) was rendered from
[`walkthrough.tape`](walkthrough.tape) driving the TUI on the demo database
with the same sandbox environment, and shows the full journey:

- `/sql <SQL>` — run bounded read-only SQL directly against the active
  profile; the result is captured for the session. The recording runs the
  same duplicate-join trap the CLI journey asks: the table shows the
  multiplied `joined_rows` beside `count(DISTINCT o.id)`'s distinct counts,
  and the evidence line (`direct sql: demo · 4 rows · exec … · full
  result`) names the connection, row count, and execution id.
- `/investigation save <name>` — takes the name **positionally**, unlike
  the CLI's `--name <NAME>`. With neither `--sql` nor `--file` it saves the
  latest successful, concrete query — a `/sql` capture — on the connection
  that actually ran it. The recording saves the trap query; its transcript
  block shows the derived, hash-suffixed id.
- `/investigations` — alias for `/investigation list`; one line per saved
  investigation. The recording's list shows two rows with the same name:
  the just-saved one (`trap-query-…`, hash-suffixed) and `trap-query` — the
  imported replay target the run below uses.
- `/export --snapshot <path>` — export the result you already inspected:
  the latest `/sql` capture, held for this session only, with **no query at
  all** (`Exported 4 row(s) to … from snapshot exec …`). The plain
  `/export <path>` re-runs the last query for a fresh read instead.
- `/report [--rows N] [--overwrite] <path>` — write a shareable Markdown
  report of the latest `/sql` capture: the exact SQL and its provenance by
  default, rows only with `--rows`. It never queries a database and never
  uploads anything. The recording writes `trap-report.md` (rows omitted).
- `/investigation run <id>` runs **in the background**, like a `/sql`
  query: the status bar names the investigation and offers `Esc to detach`
  while it runs, and when it finishes the replayed result renders as a
  table — the same presentation a direct `/sql` result gets — with the
  evidence line (`saved investigation: demo · 4 rows · exec … · full
  result`) beneath it. **Esc detaches a running
  replay** — detach, not cancel: the worker keeps going and the detached
  result is simply discarded. The replay in the recording targets
  `trap-query`, an imported twin of the same trap SQL: the tape cannot type
  the hash-suffixed id `save` generates at run time, so the tape's hidden
  phase stages the definition under a fixed id through `investigation
  import` — the same import the CLI journey performs — and the replay maps
  it with an explicit `--connection demo`, exactly as the CLI journey's
  second environment must. The detach itself is described, not shown: the
  local demo query finishes far too fast to detach from.

## What the walkthrough proves

- The whole journey runs with **no provider and no network** — discovery,
  raw SQL, saved investigations, and replay are local, bounded, read-only
  operations.
- A saved investigation travels as a **definition only**: exact SQL,
  metadata, referenced tables. No credentials, no rows, no binding.
- A definition never runs by accident: the receiving machine must map it to
  a connection explicitly, the mapping binds only after a successful run,
  and a changed target or schema is refused before anything executes.
- The evidence line ties every replay to a connection, a row count, and an
  execution id — the same evidence `investigation run --report` records in
  its shareable Markdown report.