# Job capabilities

Jobs are always available. `[coding] enabled` was removed, and a config that
still sets it fails to parse. `[agents] max_concurrent_model_requests` accepts 2
through 4 (default 4), in both bounded and uncapped modes. One slot belongs to
the coordinator, leaving at least one usable child slot;
`[agents] max_children` bounds how many helpers run at once (default 3, maximum
3). Both are top-level settings, beside the registry they bound.

The coordinator (the root) works directly in one managed working checkout `W`.
By default it advertises `read`, `write`, `edit`, `glob`, `grep`, `shell` and
`checklist`, plus `notebook_edit` where the checkout holds a notebook,
`submit_plan` in plan mode, the background shell readers after a background
run, and every configured MCP tool. `list` and `delete` are composed but
withheld — `shell` covers both — and `web_fetch`, `web_search` and `helpers`
appear only when the request or configuration gives evidence for them (see
below). Naming a withheld tool in `[tools] allow` advertises it again. A job
that needs no help does the work itself. There is no
coordinator mode or setting: the `coding.coordinator_mode` key existed only in
unreleased builds and is gone, and a config that still sets it is refused as an
unknown key like any other. Delegation is the coordinator's own choice.

A `read` selects at most a 2,000-line window and retains at most 256 KiB of
the file per call; a larger file is served truncated with its whole-file
digest, and the result still passes through the model's bounded result cap.
Every text read returns its lines numbered in `content` (`   123→text`, a
prefix that is not in the file), sent once. The whole-file digest an `edit`
precondition states is unchanged. A `read` of an image — `.png`, `.jpg`, `.jpeg`, `.gif` or `.webp`
whose bytes carry that image's own signature — serves the image itself beside a
`content` naming its media type and size, never a text view of its bytes; an
image over 3 MiB is refused whole rather than truncated, and a line range does
not apply to one. An `edit` replaces one anchored string, or — with
`replace_all: true` — every non-overlapping occurrence of that one anchor,
resolved against the one file snapshot and published once, with the count
reported as `replacements`; the default keeps the exactly-once refusal. With
`edits` it applies up to 32 `old_text`/`new_text` pairs against that one
snapshot in a single call. A stale `expected_digest` is re-anchored onto the
current content — with a `note` saying the file changed since it was read —
when the anchor still resolves under the call's own count rule: exactly once,
or at least once with `replace_all`. Zero or several matches keep the
moved-anchor refusal, a stale `expected_size` alone never refuses, and the
commit still re-verifies the digest it resolved against, so a concurrent
change between the tool's read and the write refuses. A zero-match edit names
up to three nearest regions of the current file (`lines a-b` and their text,
bounded at 2 KiB) so the anchor can be corrected without a fresh read; the
hint is skipped for a file over 1 MiB or an anchor over 4 KiB. A successful
single-file shell view inside the workspace — `cat`, `head`, `tail` (not `-f`),
`sed -n '<a>,<b>p'` or `nl` — is a read witness for that file's current
content, exactly as a `read` is; anything compound, redirected or piped is
not. A `read`
whose content is exactly the digest this run's last write or edit of that path
produced is still served whole, with a one-line `note` saying it is unchanged
since that edit. An `edit` result carries `changed_excerpt`: the changed lines
numbered as a read numbers them, with a few lines of context and bounded at 40
lines / 4 KiB per hunk and 12 KiB per result (`... (cut)` marks a trim). A
`write` result carries a short numbered head of the file it wrote.

A `read` of a notebook — a `*.ipynb` whose bytes carry a `cells` array beside a
numeric `nbformat` — shows the notebook rather than its JSON: one block per cell
with its index, type, id and source, then each output's own text. Each image
output is sent to the model as the image itself — the same image block a `read`
of an image file builds, so its base64 never appears in `content` — and an image
output that is not sent stands in as a short `[image output not sent (cell N,
<reason>): <mime>, <n> bytes]` line naming the cell and why. At most 200 cells
are shown and the rest are counted in `[N more cell(s) not shown]`, one cell
contributes at most 8 KiB of output text, and one read carries at most 8 images
(one over the 3 MiB image bound, or past that count, is a placeholder line). A
file named `.ipynb` that is not a notebook keeps the text path, and a
line range always reads the file's own raw lines. `notebook_edit` changes one
cell — `replace`, `insert` or `delete`, naming it by `index` or `cell_id` —
under `edit`'s own rules: the same write scope, the same `expected_digest`
precondition and stale-read refusal, the same checkpoint capture and redaction
guard, and the same approval path. Cells the call does not name keep their exact
bytes, and the file that lands is valid nbformat JSON.

`glob` serves its matches newest-modified first — ties by path, and an entry
whose filesystem reports no usable time sorts last — so the first hit is the
path changed last and the 500-match cap keeps the newest rather than an
arbitrary slice. An optional `type` of `file` or `dir` keeps one entry kind;
omitted serves both, and a `dir`-only call never reads file content.

`grep` reports what it actually served. `output_mode` picks the shape of each
entry: `content` (the default) one matching line, `files_with_matches` one path
per matching file, `count` one `{path, count}` per matching file — and
`matches_found`, `total_matches` and `omitted` count matching lines in the first
mode and matching files in the other two, so coverage stays true in every mode.
In `content` mode `context` serves up to 10 unchanged neighbouring lines around
each hit, each marked `context=true` and never counted as a match, and
`multiline` matches the pattern against the whole file instead of each line; the
two cannot be combined, and that combination is refused rather than silently
dropping one. `head_limit` caps the entries served, never above the 500-match
ceiling, and `omitted` still counts the rest. An optional `type` keeps one
language's extensions by ripgrep-style name (`rust`, `py`, `md`, …); a name
outside that fixed list is refused with the list itself, never answered as an
empty search.

Every workspace tool takes a path relative to the root, and an absolute path
under the root is accepted as the same file; an absolute path outside it is
refused, naming the root. The advertised schemas state the filters the engine
already bounds — `glob` and `grep` `type`, and `grep` `multiline` and
`head_limit` — and a literal `grep` that finds nothing while its pattern holds
regex metacharacters carries a one-line note to pass `regex: true`.

The root runs shell commands whenever the job's approval mode is not `never`;
`[tools] allow` is the one switch that takes `shell` away, and it takes it from
the root and every helper at once. A call runs on the host by default and
asks the operator per call under `ask`, or runs in the sandbox when the call
sets `sandbox: true`. A headless job (`--non-interactive`, or no terminal)
defaults to `never` and advertises no `shell`; under `ask` a host call it
cannot ask about runs in the sandbox and its result says so in `lane_notice`,
and `--approval-mode bypass` is how it reaches the host instead. See
`docs/commands.md` for the lanes, the conditions and the sandbox's rules.

A command is capped at 64 KiB. A foreground call waits up to its `timeout`
(default 120000 ms, ceiling 600000). Where the surface advertises the
background family, a call that hits that timeout is moved to the background
instead of killed: it answers with the
run's `id`, `status: "running"`, `timed_out: true` and a notice to read it with
`shell_output`, and the output written before the move is the first read's. If
all four background slots are held, the run is killed and the result says
`timed_out` with no id.

`run_in_background: true` starts the same call detached and answers at once
with an `id` and `status: "running"`, so a long `npm install` or a watch-style
command does not hold the turn. When a run exits, the loop appends one short
notice to the next request — after that turn's tool results, one line per exit
and no command output — naming the id, the exit code or kill, how long it ran,
and how many output lines it wrote. `shell_output`'s first read of an id
answers at once with the output written since the last read, the status and the
exit code. A read that follows a silent read of the same live run waits inside
the call for new output or the exit — 30 s, then 60 s, then 120 s for further
silent reads, clamped to the job's remaining wall clock; any output or the exit
resets that sequence. A live run's result says it is still running and that the
exit will be announced. A headless job whose model replies without a tool call
while a run is live does not finish and opens no request: it waits one
request-deadline slice at a time, bounded by the job's remaining wall clock, for
the next exit, then sends one request carrying the notice. When the wall clock
is spent it finishes as before, and the existing cleanup kills the runs. An
interactive turn ends as it always did, the notice rides the user's next
message, and the TUI shows the same line in its activity transcript.
`shell_kill` ends the run's
whole process group. At most four runs are live per shell at once; a fifth is
refused with a message naming `shell_kill`, never queued. A run needs no approval of its own: the start takes exactly the lane
decision a foreground call takes — `lane_notice` included when a headless `ask`
call is rerouted to the sandbox — and the readers reach only runs that shell
started. The coding root, its helpers and the interactive session advertise the
family; each run is killed and reaped with its surface, so a job, a helper or a
session that ends leaves no process behind. A `saya run` step's shell lives for
one call and advertises no background family.

Every shell result the model reads — a foreground call or a `shell_output`
read — is capped at 30,000 characters across stdout and stderr. A larger
result keeps both ends and carries a marker naming how many bytes were
elided and the file that holds the full captured output. That file is written
under the job's private state directory, never inside the workspace, and is
bounded per file and per job. The marker names the path and says to read it
with `read`/`grep`: this job's own tools reach exactly that spool directory,
under the existing read caps, and no other session's. A surface with no job
state directory — an interactive session's shell, a `saya run` step — saves
nothing, and its marker says so.
The result's `truncated` flag is set for either cut, the capture's own ring or
the inline bound, so a dropped middle is never silent. The shell description
asks for the narrowest relevant test first, a full suite only when the task
asks for it or the change is cross-cutting, and says that a long result comes
back as its head and tail with the exit code.

## Fetching a web page

The root and its helpers advertise `web_fetch` in the `ask` and `bypass` modes
only when the request names an http(s) URL or a `[web_search]` backend is
configured; `web_search` is advertised only where that backend is configured.
A request that names no URL, under a configuration with no backend, reaches no
web tool. `web_fetch` is
one https GET of a `url`, answered with the final URL, the status, the server's
content type and the body as text — an HTML page converted to readable text,
anything else passed through unchanged. It is the session's own `http_fetch`
egress, not a second client: the https-only gate, the refusal of loopback,
private, link-local and ULA addresses (both written as a literal and resolved,
re-judged at every redirect hop), the wall clock and the redirect bound are the
session's, unchanged, and no credentials or cookies are sent. `max_bytes`
(4 KiB to 60 KiB, default 24 KiB) bounds the *converted text*, and a longer page
is cut head-and-tail with the elided count marked — never refused for its size.
Consent follows `shell`: `ask` puts the URL to the operator once per call,
`bypass` fetches without asking, and a headless `ask` job is refused with a
reason naming `--approval-mode bypass`. `read-only` and `never` advertise no
fetch at all, so those modes reach no host.

## Searching the web

`web_search` answers a `query` with up to `max_results` results (1 to 10,
default 5), each a title, a URL and a snippet. `allowed_domains` keeps only
results on those domains and their subdomains; `blocked_domains` drops them.
The filters are applied by Saya to the results, so the answer reports
`filtered_out` — how many the call's own filter removed — and a model that sees
two of three results is told a filter took the third.

Saya runs no search service of its own, so the tool is **off until configured**:
with no `[web_search]` table, no surface advertises it. The table names a
`provider` (`searxng` or `brave`), the `endpoint` to ask, and — for `brave`, as
a secret *reference* like every other credential here — an `api_key`. The table
is protected: a project's `.saya/config.toml` cannot configure it, retarget it,
or turn it on.

The request goes to that endpoint and nowhere else, over the same egress as
`web_fetch` — the same policy, resolve-then-deny check, wall clock and byte
bounds — with the redirect bound closed: a search that answers `302` is refused
rather than followed, because the model supplies words and never a destination.
Consent, the advertised modes and the refusals are `web_fetch`'s, worded for a
search; a configured credential reaches the wire and nothing else, and a
reference that does not resolve fails the call naming the reference, never a
value.

## Delegation with `helpers`

`helpers` is the whole delegation surface, advertised only when the request
text asks for it — it names a helper, subagent or sub-agent, parallel or
fan-out work, or delegation. A `[tools] allow` entry naming `helpers` also
advertises it. One call is one helper, and several
`helpers` calls in one turn run at the same time, bounded by
`[agents] max_children`. The call waits for its helpers and returns a redacted,
capped summary per helper — its outcome, its failure class and its answer —
carrying no authority. A helper's own failure is reported in that summary and
does not fail the job.

Every helper works in the root's own working checkout `W`. There is no
per-helper checkout, no merge step, no path lease and no `records/merges.jsonl`:
a helper's own writes are already in the checkout the operator reviews. The
job-level write-path ceiling is gone, so the root and a helper may change any
file in the checkout; operator review and apply is the check. A helper cannot
delegate further.

`subagent_type` picks the helper's shape. `explore` reads only: it can read and
search, and it never edits. `general` implements one piece. An `explore`
helper's `shell` is sandbox-only with nothing writable, whatever the job's mode;
a `general` helper's follows the job's Ask/Bypass rule. Under the mode `never`,
or with `shell` removed from `[tools] allow`, a helper gets no shell at all.

The types are a registry, so a trusted `[agents]` table adds one — the same
table states how many helpers one job may run at once:

```toml
[agents]
max_children = 3             # helper children running at once

[agents.reviewer]            # a new type starts read-only
role = "analyst"

[agents.builder]
role = "implementer"         # ...and this one writes, inside the job's ceiling
```

`role` is `analyst` (read-only) or `implementer` (may write); `allow_write`
narrows an implementer to read-only, and `model` gives that type's helpers a
model of their own — they ask the job's own endpoint for it, and without it they
send exactly the model the job itself sends, so the request is the one they
always sent. A declared entry may only narrow a type
that already exists — `[agents.general] allow_write = false` makes `general`
read-only — and a widening entry is a typed resolve error rather than a silent
no-op. A type the job does not advertise is refused by name, with the types it
does. `[agents]` is protected, so a project-layer config can neither add a type,
retarget a built-in, nor raise either bound. The shape is never a
grant: every child's authority is still narrowed from the job's own scope, so a
configured `implementer` writes only where the job already could.

Each assignment is capped at 32 KiB, so a coordinator references requirement
files by path instead of restating or paraphrasing them. A longer assignment is
refused before any child, checkout, history or budget record exists, with a
message telling the coordinator to shorten it; the job continues. The root
prompt and the tool schema state the same limit.

## Run a job plan-first

`--plan` on `saya start` (or `/start`) starts a job with its write
surface closed. The first request advertises only the read-shaped part of the
job's tools — `read`, `glob`, `grep` — plus the job-local `checklist`
and one new tool, `submit_plan`. The write family, `list`, `delete`, `shell`,
`web_fetch`, `web_search` and `helpers` are not advertised at all, so a call to
one is refused as an unknown
tool rather than offered and then denied. Plan mode needs a write scope: state
`--write`, or the start is refused.

`submit_plan` takes one `plan` string, at most 2048 characters, and is the whole
of the question: it puts the plan to the operator and ends that turn. Approval
opens the rest of the job's tools for the rest of the run; a refusal leaves them
closed, and the model may revise the plan and submit again. The gate is an
approval, not an authority — it grants exactly the ceiling the job already had,
and read-only SQL and the write scope are untouched.

Plan mode is opt-in, and it needs someone to answer. A job without `--plan` is
unchanged in every respect: the same advertised tools, the same prompt bytes,
the same `request.json`. A headless run — `--non-interactive`, a scripted
`saya start`, an automated driver — has no operator, so `--plan` there
is refused at start with a message naming the interactive panel, exactly as
`request_clarification` is withheld where nobody can answer. `[tools] allow`
still governs, and a `[tools] allow` list that leaves `submit_plan` out is
refused at start for the same reason: the posture would have no way to end.

The session's `plan` permission mode (`/mode plan`, or the Shift+Tab cycle) is
the same gate on the same tool: it advertises the read-shaped surface plus
`submit_plan` and hides the write family, the `shell` and the MCP servers, and
the approval is put to the session's own surface — the TUI's approval modal or
the REPL's terminal prompt. The one difference is that the session's approval
belongs to one plan: leaving `plan` closes the gate again, so re-entering it
asks afresh.

## Work in place

`saya start <id> <task>` edits the stated folder in place by default, including
a Git working tree: tool edits land in the working tree and are captured as
before-images, and no managed checkout is created. `[job] root_mode =
"managed"` restores the managed-checkout default; `--managed` selects it for
one job, and `--in-place` still forces the folder even when the configured
default is managed. A folder that is not a Git working tree always binds in
place. The job keeps an edit-time baseline marker in its private records: the
first bytes each tool saw at a path, bounded at 4,096 paths, 8 MiB per file and
512 MiB in total, with no whole-tree walk and no file-count cap. The job's
durable repository identity is the folder's local id (`loc-…`), and `list`,
`show`, `notes` and preferences all resolve through that path; in a Git
worktree `list` also shows the jobs started under the repository id (`repo-…`),
so both roots are visible. A session started in a plain folder binds it as the workspace,
so `/start` reaches the same in-place path; a session in a Git worktree says at
startup that edits land in the folder and `/job rewind` undoes tool edits. A
dangerous root still binds nothing.

A `--managed` start (or a configured managed default) that cannot use a
checkout — an unborn or conflicted index, a sparse or partial clone, an
included config or a filter attribute, a nested repository, an unsupported
worktree, or an inventory/snapshot bound — falls back to the same in-place
start and states one line on stderr: `managed checkout unavailable (<reason>);
working in place — tool edits can be rewound with saya job rewind`. The start
result and `job show` carry `"managed_fallback": {"reason": …}`, and the reason
is recorded privately under the job. Cancellation, a timeout, an I/O failure, a
changed repository, and a state-directory error stay errors.

A job with no `--write` path, and an in-place job that states no scope, composes
the read-only workspace tools (`read`, `glob`, `grep`) over the stated folder
and can answer from what it reads. It composes no workspace write tool and no
shell, so writes stay refused.

An in-place job with a `--write` scope gets the full direct-mode workspace
toolset rooted at the folder, and the scope is enforced at the tool boundary:
`write`, `edit`, `notebook_edit` and `delete` refuse a path outside it, and
`--write .` is the whole folder. An in-place job that selects a write tool but
states no `--write` is refused at start; the exception is `saya -p` under an
edit-capable consent, where no `--write` means the whole workspace (`.`).
`saya job review` shows the folder's
changed paths — the job's own edits from its before-image store, plus the
changes Git status reports for the shell — with evidence binding, and reports
`shell_changes` separately (or `unrecorded` with a reason). `saya job apply`
finalizes it: apply copies nothing, it verifies the folder is still exactly what
was reviewed and records a private acknowledgement, so a second apply reports
`already_applied` and a changed folder is refused as stale. The job's outcome
states the folder, the `edit_time` baseline, `recorded {files, bytes,
overflowed}`, the changed paths, finalize state and the `--baseline` undo, with
a notice only when the job changed something; shell-made changes are reported,
never rewound. The filesystem root, the home directory, and saya's own state
directory are refused at start.

Every file the job commits in place is recorded in a private journal: path,
digest and turn. `saya job review` marks a changed path the journal does not
account for as `foreign_changes`; a rewind reports such a path as
`foreign_paths` and leaves it exactly as it stands unless `--force` is passed.
Without a journal there is no basis, so nothing is called foreign.

The baseline is an edit-time marker: the folder's identity and snapshot id,
with no inventory, so a large folder starts without a walk. What the job
changed comes from the before-image store, bounded at 4,096 paths, 8 MiB per
file, and 512 MiB in total. A path over a bound is recorded as `unrecorded`
with its reason: the edit proceeds, the path is labelled not recorded, and
`--baseline` reports it as `not_restorable` rather than putting bytes back.

## What a managed checkout snapshots

A Git job's baseline is the working tree at start: tracked content plus
untracked files, minus ignored paths. Untracked credential-shaped paths and
configured private prefixes are omitted, and the plan names each omission and
its reason. The credential shapes are one fixed set, shared by the snapshot
and the in-place workspace predicates:

- directory components `.aws`, `.ssh`, `.azure`, `.gnupg`, `.kube`, `.codex`,
  and `.saya`;
- exact file names `.env`, `.netrc`, `.npmrc`, `.pypirc`, `credentials.json`,
  `id_rsa`, and `id_ed25519`;
- the `.env.*` family;
- the `.pem`, `.key`, `.p12`, and `.pfx` extensions.

Names that merely contain `secret` or `credentials` — a module such as
`secrets_manager.py`, say — and files under a generic `.config`, `.claude`, or
`.docker` directory are ordinary repository content: they are snapshotted, not
omitted. A **tracked** credential-shaped
path is repository content: it is included, and the plan, the job events, and
`saya job show` (`snapshot_warnings`) carry a non-fatal
`sensitive_tracked_included` fact naming the path, never its content. At most
64 such facts are listed; `snapshot_warnings_truncated` states how many
further paths the capped list does not name. An in-place job's own omissions
are reported under `in_place_omissions`, never as snapshot warnings. Env
stripping, redaction, and read-only SQL are unchanged.

A symlink whose target is relative and resolves inside the repository is
snapshotted as a link: its target string is its content, bounded to 4 KiB and
counted toward the file bounds, and it materializes as a symlink. An absolute
target, a target that escapes the root, or one that enters `.git` refuses with
a message naming the path and the reason; submodules and other non-regular
types refuse as unsupported.

## Narrow the job's tools

`--tools name,name,...` on `saya start` (or `/start`), and `[tools]
allow` in trusted configuration, narrow a job's tool surface like every
other agent surface. The list only removes: the job composes its tools exactly
as it would without it and then keeps only the names listed, so `shell` stays
hidden under mode `never`, or with `shell` left out of the list, however the
list is spelled. `shell_output` and `shell_kill` are parts of `shell`, not separate
names: a list that keeps `shell` keeps all three, and a list without it keeps
none of them. `web_fetch` and `web_search` are names of their own: a list that
omits one takes that tool away, and no list can hand either to a mode that
reaches no host, or `web_search` to a run with no `[web_search]` configured.

On the CLI the flag is the top layer, resolved before the job starts. Inside a
session `/start --tools …` can only narrow: a name the session does not
advertise is an error that lists them, and no flag hands the job the session's
own list.

The job persists the set it started with, so a resume and a follow-up run
exactly what the operator approved at start, and a narrowed set is stated in
the start proposal. A helper runs the job's set minus `helpers` — a helper can
never delegate. Database tools are not governed by the list: they follow the
`--database-profile` selection.

## Tools from MCP servers

A trusted `[mcp_servers.<name>]` table names a program to run over stdio —
`command`, `args`, and an optional `env` of secret *references* (never
literals). When one is configured, the servers start once for the session or
the job and each tool they advertise joins the surface as
`mcp__<server>__<tool>`. Nothing starts without the table: the default is off.

The table is protected, so a repository's `.saya/config.toml` cannot add a
server, replace one, or retarget one at a different program. Remote (HTTP)
servers are not supported in this release; a table that states a `url` is
refused with a message naming `command`.

**A server's tool definitions are untrusted input.** What the model reads is
the server's own words, prefixed with `[MCP server "<name>"] ` so the origin is
legible at the point of use, and bounded: 64 tools per server, 1024 characters
per description (truncated, with `…` where the rest was), 16 KiB per input
schema, 4 MiB per frame. A tool whose name a provider's function field cannot
carry, whose schema is over the byte cap, or whose exposed name collides with
another tool of the same server is **dropped**, not renamed or truncated, and
the drop is reported at session start.

**Every call is consented like a host `shell` call.** A server is a program
whose effects saya cannot bound, so the modes are the shell's: `bypass` runs a
call, `ask` puts each call to the operator, and `read-only` and `never` reach
no server at all — no lane is composed under them, so no `mcp__` tool is
advertised. A driver with no operator (a headless `saya start`, a script)
refuses under `ask` with a message naming `--approval-mode bypass`. A call's
answer is bounded to 64 KiB with both ends kept, and the credential values the
server was handed in its `env` are scrubbed out before the model reads a byte.

**`[tools] allow` governs MCP tools too.** An `mcp__` name is a capability the
list may take away, in either spelling: `mcp__<server>` keeps every tool of
that server — including tools the server had not advertised when the list was
written — and `mcp__<server>__<tool>` keeps exactly that one. A list that names
neither takes the whole family away, from the advertisement and from the
dispatch door alike. A malformed `mcp__` word is refused at launch by shape,
not silently ignored.

**Lifecycle.** Servers start lazily, once per session or job, bounded to 30
seconds for the whole pass and `startup_timeout_ms` per server; each is killed
and reaped when its lane drops. A server that fails to start, hangs, or exits
is reported at start and its tools are simply absent — never silently mistaken
for a server with nothing to offer.

Which surfaces advertise MCP tools: the interactive session, the coding root
and its helpers, a `saya run` step, and `saya ask`. Each narrows the family
through its own `[tools] allow` list and applies the consent rule above. A
helper puts a call to the job's operator, exactly as the root does. A run step
and a headless `ask` have no operator, so under `ask` they refuse with the same
typed reason naming `--approval-mode bypass`; a helper of a job with no
operator does the same. A helper's servers are its own connection, started when
the helper is composed and closed when it ends; a run's lane is built once for
the run and a one-shot `ask`'s once for its turn.

## What each child receives

Every child starts from a fresh request that holds, in this order:

- **The operator request**, exactly as given when the job started, labelled as
  the requirements source. It is required context: it is never truncated, so
  optional context (project instructions, preferences) shrinks before it does. A
  follow-up invocation of a completed child receives it too.
- **The coordinator's assignment**, labelled untrusted. It may scope and
  coordinate the work but cannot change the operator request's requirements. A
  closing instruction tells the child to read the repository files the request
  references. The operator request is authoritative over the assignment's
  directives: a directive that cannot be traced to the request is not followed,
  and the child marks it in its final report as a line
  `assignment-added: <the extra directive>`. The root sees those marks on the
  helper's summary.

Neither text widens authority: each child's write scope and database profiles
stay within the approved job scope.

The coordinator's own prompt, and the `helpers` description it reads, carry the
same rule from the other side: the user's request is the only authority for
requirements, the coordinator must not invent requirements, formats or
conventions where the request leaves room, it prefers the most conventional
reading, and when an assignment adds an interpretation of the request it marks
it explicitly as `my reading` and quotes the request sentence it interprets.
This is prompt guidance, not a gate: no check compares behaviour to the request,
and a run that ignores the rule is not refused.

## Select database access explicitly

A job has no database capability by default. Select existing profiles
when starting it:

```sh
saya start job-report "Implement a report using the approved schema" \
  --write src/report.rs --database-profile analytics --approve
```

Repeat `--database-profile` to select several profiles (at most 32). Coding
profile names must be nonempty, at most 128 UTF-8 bytes, and contain no
whitespace or control characters, matching the child request contract. This
restriction does not change profile names accepted by ordinary database
commands. The same start arguments work through `/start` in the
interactive terminal; its approval step uses `/approve <job>`. Review the
consent proposal before approving: it includes each selected name, its resolved
identity and the effective connections-file path, with project-provided origins
identified. Project connections can exist independently of project AI
configuration trust; a familiar profile name alone is not proof of a
user-controlled origin.

The coordinator and its helpers call the ordinary database tools,
`schema_discovery` and `bounded_sql_query`, only for the job's approved
identities. The tool schemas enumerate the exact selected names so both
coordinator and helpers can discover usable choices. Child authority can only
narrow that selection. An unselected profile is refused before connecting. The
connected identity is checked before dispatch. These tools route to the
ordinary database owners: read-only SQL safety, model row caps, query timeouts,
cancellation settlement and audit behavior remain active. Queries require
`ai.allow_data_sharing = true`; row tools are hidden when it is disabled and
defensive execution checks still refuse row access. Schema discovery is
available with sharing disabled. Selecting a profile does not enable data
sharing. Database credentials are resolved through the existing secret resolver.

The coordinator and its helpers also carry the clarification ask,
`request_clarification`, which is not a database tool: it needs no connection
and no selected profile, so a job may stop and put one focused question to the
operator. It is advertised only when an operator surface can answer it — the
interactive coding panel — and never in a headless run (`--non-interactive`,
`saya start` from a script, an automated driver), where nobody can
answer and a question would block. Calling it ends the turn; the operator's
next message is the answer.

The coding owner binds relative SQLite file paths to its composition cwd
before consent, identity hashing and connection use. Resume from another cwd
with the same relative profile refuses when that resolves to a different file.
SQLite `:memory:` is unsupported. Ordinary database sessions are unchanged.

Consent and saved jobs bind the resolved identity, not only the profile name.
Changing a selected profile's identity causes resume to refuse; start a newly
approved job for the new database. Every resume still needs fresh exact
approval. Old saved requests retain no database authority. Repository text,
tool arguments and child messages cannot grant profile access.

Connection setup is bounded by the configured query timeout and job
cancellation, including connector construction. Query cancellation uses the
connector owner's existing settlement path.

## Review managed changes

Helpers write the one working checkout directly, so there is nothing to merge
and nothing for the model to inspect through a separate tool. The operator
reviews with `saya job review`, which builds an isolated candidate under
fresh approval and returns bounded change previews and source identity, and
applies with `saya job apply`, which verifies the digest and the unchanged
original tree first. There is no model-controlled apply tool, and no check
receipt gates either command: the operator's review and apply **is** the check.

A foreign candidate refuses before any review effect, leaving job accounting and
fresh approved resume usable. Failures after review starts retain ordinary
effect accounting; they are not declared harmless.

The review delta follows the checkout's own Git ignore rules: an ignored path is
not job content and never appears as a change, so an ignored build tree costs
one bounded listing rather than a walk over every generated file. A nested
repository or special file is reported under `not_included` with its reason
instead of failing the review. If a file, byte, or time bound is reached, the
delta keeps the included paths, sets `changes_truncated`, and reports
`included_files`, `included_bytes`, `not_included_overflow`; apply-back
preflight covers exactly that included set.

## Rewind the working checkout

At each turn boundary where the job's own tools changed the working checkout
`W`, the owner takes a checkpoint: the changed paths and the exact bytes each
held then. `saya job rewind <job>` (or `/job rewind <job>`) lists them,
and `saya job rewind <job> --to <turn>` — `/job rewind <job> <turn>` in a
session — restores `W` to one. Both spellings reach one operation; stating a
turn twice is refused rather than resolved.

A checkpoint covers exactly what the job's file tools wrote inside `W`: a
rewind restores file content and nothing else. It never undoes a command, a
download, a database effect, or anything outside the checkout, and it grants no
authority — every restored path is one the job's write scope already admits, and
a recorded path outside that scope is reported and never touched. A rewind is
refused while the job is running a turn, and a turn with no checkpoint is
refused by name. A rewind also reports `foreign_paths`: paths whose current
bytes the job's own journal does not account for. They are left as they stand
unless `--force` is passed, so a concurrent edit is never silently lost. After a rewind the next run's context states which checkpoint
the checkout was rewound to, so the model re-reads rather than trusting what it
wrote earlier.

`saya job rewind <job> --baseline` is the undo of last resort for an in-place
job: it restores every baseline path the job's `--write` scope admits and
removes in-scope paths added after it. A baseline over the rewind bounds is
refused without writing, and an out-of-scope path is reported and never
touched. It changes file content only.

The model can reach the same store with the `rewind` tool. It is offered from
the run's first request — before the first checkpoint it lists an empty set —
and only to the job's root: a helper and a `saya run` step advertise no rewind.
Called without `checkpoint` it lists the newest 20 checkpoints with their id,
turn, and changed paths;
`checkpoint` takes one of those ids or `last`, and `paths` restricts the
restore to named workspace-relative files. The restore is the operator
command's own code over the same checkpoints: it writes back the recorded
bytes and removes a file the run created since that checkpoint, and it never
touches a file the run did not change. It is a workspace write, so it follows
the same approval and permission mode as `write` and `edit`. The result names
the restored and removed paths with the new digest and size, so a following
`edit` can use them without another read. Files only: it never undoes a
command, a download, a database effect, or conversation history.

Bounds: 32 checkpoints per job (the oldest dropped first), 16 MiB of image bytes
and 512 paths per checkpoint, 8 MiB per file. A turn over a bound records
nothing at all rather than a partial checkpoint, and the listing says so.

## Stop a job on progress, not on spend

A job stops when it stops making progress, not when it has spent a
certain number of requests or tokens. Counts are reported, not enforced as
spend targets:

- `[limits.job] no_progress_turns` (default `8`) stops after that many
  consecutive turns with no progress. Progress is a workspace change, a shell
  outcome that materially differs (output or exit code) from the last run of
  the same command, a search that found a match, a first read of a path, a
  checklist change, or a submitted plan. A new command that wrote nothing and
  succeeded, a zero-match search, a re-read, and an identical repeated command
  are not progress. A shell command that only inspects (`cat`, `head`, `tail`,
  `sed -n`, `ls`, `find`, `grep`, `rg`) is classified like the read tool it
  stands in for.
- Consecutive **calls** without material progress are counted against the same
  `no_progress_turns` limit, so churn inside an otherwise productive turn is
  visible: the call at the limit gets one short note, and the call past it
  stops the job. The per-turn guard above is unchanged.
- `[limits.job] repeat_limit` (default `3`) stops a call that returns the same
  result digest that many times in a row. The first time the limit is reached
  the model gets one short note to change approach; the next repeat stops the
  job.
- `[limits.job] wall_clock_seconds` is optional: unset means no wall-clock
  guard, attended or not; a configured value is honoured exactly.
- `[limits.job] max_cost_usd` is optional and compares only against a cost the
  provider reported.

A progress stop ends the job cleanly: the loop takes one final turn (no tools
except the checklist) to record the best state and a summary, and the candidate
is prepared as for a normal finish. Every stop records `failure.class`
(`no_progress`, `repeating`, `wall_clock`, `cost`, or `backstop`),
`failure.limit` (for example `limits.job.no_progress_turns`),
`failure.limit_value`, and `failure.measured`, plus a one-line message naming
the key to change. Each turn that ends without progress records its count in
the job's progress report and as a durable job event; a turn that resumes
progress records the reset once, and a healthy turn records nothing. `saya job
show` prints the latest check and `saya job events` lists the per-turn events.
The request, tool-call, and elapsed-time values in `[limits.job]` are runaway
backstops only; see [configuration](configuration.md#job-limits).

## Provider usage in job status

`saya job show`, `saya job export` and the `saya start` result include a
`usage` object built from the job's durable transport spans: `requests`
(provider calls that reported usage), `input`, `cached_input` (a subset of
input), `output`, `reasoning` (only when a provider reported it),
`model_wall_time_ms` (the sum of the spans' measured durations), `cost_usd`
(only when a provider reported a cost, such as OpenRouter's `usage.cost`), and
`complete`. `complete` is true only when every recorded span reported its
input and output tokens; otherwise `incomplete` names how many spans lacked
`input` or `output`, and `read_truncated` says the bounded read (64 pages of
100 attempts) stopped before the last span. A never-dispatched attempt
consumed nothing and is not a gap. A breakdown no provider reported is
`null`, never a guessed zero; tokens and cost are never estimated.

The same view includes a `progress` object — `turns`, `last_progress_turn`,
and `progress_events` by kind — from the progress report the root loop wrote
when it finished. It is `null` for a job that recorded none.

## Interpret failures and diagnostic history

`saya job show` exposes the latest durable root failure event through
`failure`, with a safe category and child agent/invocation identity when
available. `saya job events` and `saya job export` expose the durable
events; export also includes status. The interactive coding result surfaces the
safe stop category. No provider exception text, credentials or arbitrary tool
error message is added to these causes.

Categories distinguish provider errors, provider streams that ended early,
output limits, cancellation, budget stops, diagnostic storage faults, mandatory
persistence failures and uncertain effects. A child failure retains its category
even after its summary has been read. A job's primary class is the
highest-ranked class it latched, and its secondary the next distinct one, so the
same set of failures always reports the same primary whatever order the failures
arrived in. From most to least severe:
`mandatory_persistence`, `effects_uncertain`, `admission_denied`,
`dependency_blocked`, `setup_failed`, `extension_exchange`,
`provider_stream_ended`, `provider_error`, `output_limit`, `invalid_tool_call`,
`unsupported`, `limit_reached`, `coordinator_stalled`, `diagnostic_storage`,
`progress_delivery`, `cancelled`, `budget_stopped`,
`internal`. The rank table still ranks `coordinator_no_progress` between
`coordinator_stalled` and `diagnostic_storage`, so a record an older build wrote
reads the same, but no run emits it any more: the coordinator's no-progress
bound was replaced by the ninety-six-call `coordinator_stalled` stop. Secondary
logging/cleanup health must not replace that cause or skip authoritative
terminal handling. If authoritative terminal persistence fails, the result says
state is unconfirmed: inspect durable events before resuming, rather than
assuming a terminal state was recorded. Existing output-token-limit guidance
remains actionable.

A coordinator that cannot make progress is told so instead of being left to
loop, but readiness is advisory: it names no forced step. For a job with write
paths, the readiness view reports the working checkout's facts — `w_changed`,
`changed_paths` (at most 64, with `changed_paths_truncated`), `child_failures`
(at most 8, with `history_truncated` when the walk that read their classes did
not reach the end of the job's completion history), the live children and one
`suggestion` — and leaves `next_action` unset. A job without write paths has no
candidate to prepare.

A direct root that changes nothing for too long is still stopped. If ninety-six
consecutive coordinator tool calls leave the job's durable progress unchanged
while no child schedule is running, the owner ends the root run with the failure
category `coordinator_stalled`, whatever the suggestion says. Any call that
advances the job resets the count. The stop applies or discards nothing and
keeps the candidate, receipts and effect accounting as they were; the job's
primary cause stays the highest-ranked class it latched, so this stop can
displace an earlier, lower-ranked cause but never a higher-ranked one. A job
without write paths is never stopped by this bound.

## Prepare the candidate and read readiness

At the end of a run the owner prepares the candidate once from `W`, when `W`
changed, no child is live and no receipt exists for the current state. A
headless `saya start` has no operator waiting on a candidate, so it defers this
preparation; `saya job apply` reviews and applies `W` directly.
`candidate_ready` comes only from a receipt for that state; the readiness a
status shows is advisory and never forces a next step. If the end-of-run
preparation produces no ready candidate, the final status names a closed reason
— `nothing_to_prepare`, `children_active`, `candidate_state`, `cancelled`,
`busy` or `internal` — with one fixed sentence, and the job is not failed. A
receipt that cannot be read is never reported ready: the advisory cache is keyed
on the receipt's own bytes, and it is not used at all while a command the root
started can still write `W` — a running command changes `W` on its own schedule,
so the facts are recomputed until it ends.

Ordinary diagnostic quota rollover keeps recent whole records within fixed
bounds and does not stop valid execution. `diagnostic_history` distinguishes
`complete`, `truncated`, and `unknown`; legacy journals and interrupted storage
can leave exact loss counters unknown. A job can finish with incomplete
diagnostic history. Export places retained diagnostic records and their summary
in `artifacts.diagnostics`, separately from authoritative events and their page
completeness. Reading only the active tool-events file no longer reads the full
retained history. Each row for a call that ran also carries the call's bounded,
redacted transcript — its id, the arguments the model sent, the result the tool
returned and how long it took — so the evidence names which call touched which
file, under a per-job ceiling that ends with one explicit marker row. See
[diagnostic retention](coding-tool-diagnostics.md) for segment ordering, storage
bounds, the transcript's own bounds, crash/resume behavior and privacy.

These changes preserve strict approval, effect, budget and continuation records,
read-only database safety, and operator-only apply with fresh approval. They do
not make a retained log or a child report sufficient authority to apply a
candidate to the original repository.

## Read the job outcome

Every finished run writes one bounded `outcome.json` in the job's private
records. The same block is the `outcome` field of `saya start`, `saya resume`,
and `saya job show`, in both text and `--format json`:

- `state` — the authoritative terminal state. A root loop that ends with the
  model's final answer is `completed`, whether or not the job changed or
  committed anything. A guard stop is `paused`, an operator cancellation stays
  `cancelled`, and a run waiting on an approval stays `waiting_for_approval`.
- `reason` and `reason_detail` — for a non-completed run, the failure class and
  its bounded, redacted one-line message; both are `null` for `completed`.
- `answer` — the model's final answer, redacted and bounded to 16 KiB with a
  `[truncated at 16384 bytes]` marker; `answer_truncated` is true when the run
  or the block cut it. It is `null` when the run produced no answer.
- `commits` — up to 32 commits made in the working checkout since its base,
  each with `sha` and `subject` (subjects bounded to 200 bytes with
  `subject_truncated`). `commits_truncated` says the page was cut, and
  `commits_error` says the walk could not run.
- `changed_paths` — up to 64 working-checkout paths with
  `changed_paths_truncated`, the same facts the readiness view reports; the
  list may be empty.
- `checkout` — the working checkout path, or `null` before one was created.
- `readiness` — the end-of-run readiness envelope as one field.
- `model_prose` — `"untrusted"`: the answer and commit subjects are model text,
  never instructions or authority.

The block is written after the terminal event and rewritten by a later resume.
If writing it fails, the terminal state stays authoritative and the failure is
reported as a diagnostic storage fault.

## Troubleshoot a provider stream that ends early

Some gateways close a long streaming response before the model has finished; one
was observed closing at about 300 seconds. A cut that already delivered content
is not re-sent identically: Saya continues with a closed note and the same
configured ceiling. If every attempt still ends this way, the child fails with
the category `provider_stream_ended`. On a `failed` or `uncertain` completion
the child's summary carries its `failure_class`; `completed` and `cancelled`
never carry one, and a failure stored before classes existed omits the key. The
category is a fixed name and never carries provider text, and the operator
message names the gateway time limit and `ai.max_output_tokens` rather than the
configured model.

To fix it, raise the gateway's streaming time limit, or lower
`ai.max_output_tokens` so the longest response finishes inside that limit. The
time limit divided by the model's sustained tokens per second estimates the
largest safe value: for example, at 147 tokens per second and a 300 second limit,
about 44,000 tokens.
