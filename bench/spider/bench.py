#!/usr/bin/env python3
"""Runs and scores saya against Spider 1.0 and Spider 2.0-lite.

    python bench/spider/bench.py run    [--suite all] [--workers 8] [--session]
    python bench/spider/bench.py score  [--session]
    python bench/spider/bench.py report [--session]

Run and score are separate because they fail for different reasons and take
different lengths of time: a run is hours of model calls, scoring is minutes of
result comparison, and re-scoring must never mean re-running.

Results are appended to `results.ndjson` as each question finishes, and a
completed question is skipped on restart. A run of this length will be
interrupted at some point; losing it would be the expensive kind of mistake.

Two readings are reported for every suite:

    last          the final statement the agent ran
    designated    the statement it nominated as its answer

Spider 2.0 questions are multi-step and the last statement is frequently an
exploratory probe, so the two readings differ by a lot there and barely at all
on Spider 1.0. Designation is saya's own protocol, not a Spider concept — any
published figure has to say which reading it is.

The session arm — `run --session`:

By default every question is one independent `saya ask` process with no
conversation behind it, which measures one-shot text-to-SQL. `--session` opts
into measuring the session-based product instead: one conversation per
database, questions in a fixed order, so each question sees the turns before
it. The default path is untouched — this is an additional arm, not a
replacement, and the two never share a results file, a state store, or a
session directory (session-mode paths carry a `-session` suffix), so a row's
`mode` field and the file it lands in both say which arm produced it.

How the session is built. `saya ask --continue` cannot carry a conversation:
`ask` runs one turn with empty history and never touches the session store;
`--continue` and `--resume` are honoured only by the bare `saya` REPL. Session
mode therefore spawns the bare REPL per question — one process, one turn —
with the question piped on stdin as a single line and `--continue` set. Each
process appends its turn to the session the previous process saved, so the
questions of one database really do accumulate into one conversation. What
this buys and what it costs, decided and recorded here:

- Ordering: questions run in the harness's existing per-database corpus order,
  the same order the default arm uses, so the arms differ only in the mode.
- Boundary: one session per database. A session persists its profile and
  attached profiles, so a suite-wide session would answer every later database
  from the first one's connection — per database is the only correct boundary,
  and it matches how the state store is keyed.
- Failure coupling: no reset. An errored turn is not recorded into the
  conversation, so a transport failure cannot poison later turns; a
  successful-but-wrong turn can, and that is the product behaviour under
  measurement. The raw per-question streams show where a database collapsed.
- Resume: a session is saved after every completed turn, so a run that dies
  mid-database resumes with `--continue` into the surviving conversation —
  the interrupted turn is gone (it never completed) and its question is
  re-asked, honestly labelled as a continuation. Whether to continue is
  decided fresh before every question from what the store actually holds, so
  a stretch of failed turns never leads the next row to claim continuity it
  would not have (`session_continued: false`), and a turn that completed but
  whose row was never written (a crash between turn and row) still counts,
  because it really is in the conversation. One harness process per arm at a
  time: two concurrent runs on the same arm would merge their questions into
  one conversation while each claims its own continuity.

Two restrictions, for honesty rather than capability. `--candidates` above 1
is refused in session mode: the session surface has no multi-attempt
orchestration, so a row claiming three candidates would lie. And questions
are whitespace-folded to one line, because the session surface reads stdin
line by line — only the reference documents Spider 2.0 attaches to 55
questions carry newlines, and folding them keeps the words while losing the
layout. Finally, continuity depends on saya's own privacy rule: turns that
queried the database are fed back only when the arm's config sets
`ai.allow_data_sharing` (the generated Spider config does; Spider corpora are
public). With sharing off the session arm measures recall and the state store,
not conversation — the run warns when it sees that.

Environment:

    SAYA_AI_API_KEY   required — the model API key
    SAYA_BQ_KEY_FILE  path to a BigQuery service-account JSON (v2bq only)
    SAYA_BQ_PROJECT   the project that runs and is billed for the jobs
    SAYA_BENCH_BIN    saya binary (default: target/release/saya)
"""
import argparse, json, os, re, subprocess, sys, threading, time, urllib.request
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(ROOT, "..", ".."))
CORPUS = os.path.join(REPO, ".bench", "spider")
OUT = os.path.join(CORPUS, "results")
# One results file per arm. Two arms that differ only in a setting must not
# share a file: the resume logic keys on the question, so the second arm would
# skip every question the first already answered and silently measure nothing.
ARM = os.environ.get("SAYA_BENCH_ARM", "default")
RESULTS = os.path.join(OUT, f"results-{ARM}.ndjson")
SCORED = os.path.join(OUT, f"scored-{ARM}.ndjson")


def results_path(session=False):
    """The results file for an arm. Session mode gets its own file: two arms
    that differ in any measured setting must not share one, because resume
    skips keys already present and the second arm would silently measure
    nothing. The suffix makes that sharing impossible to arrange by accident."""
    return os.path.join(OUT, f"results-{ARM}{'-session' if session else ''}.ndjson")


def scored_path(session=False):
    return os.path.join(OUT, f"scored-{ARM}{'-session' if session else ''}.ndjson")
HOME = os.path.join(CORPUS, "home")
GOLD_URL = ("https://raw.githubusercontent.com/xlang-ai/Spider2/main/"
            "spider2-lite/evaluation_suite/gold/exec_result")

SUITES = ("v1", "v2local", "v2bq")
LABEL = {"v1": "Spider 1.0 dev (SQLite)",
         "v2local": "Spider 2.0-lite local (SQLite)",
         "v2bq": "Spider 2.0-lite (BigQuery)"}

sys.path.insert(0, CORPUS)          # evaluate_utils.py is fetched by setup.py


def _stub_cloud_sdks():
    """Let the benchmark's comparator import without the cloud SDKs.

    `evaluate_utils.py` imports the BigQuery and Snowflake clients at module
    scope for the benchmark's own runner, but the only thing used here —
    `compare_multi_pandas_table` — is pure pandas. Left alone the import
    raises, the scorer swallows it as a failed comparison, and a whole suite
    reports 0% while looking like a genuine result.

    Stubbing keeps their file pristine, so an upstream edit cannot silently
    defeat a text patch. Only these names are stubbed: anything else missing
    should fail loudly rather than be papered over.
    """
    import types
    absent = ("google", "google.cloud", "google.cloud.bigquery",
              "snowflake", "snowflake.connector")
    for name in absent:
        try:
            __import__(name)
            continue                      # really installed; leave it alone
        except ImportError:
            pass
        module = types.ModuleType(name)
        module.__path__ = []
        module.__getattr__ = lambda _attribute: None   # any symbol, unused
        sys.modules.setdefault(name, module)
    for parent, child in (("google", "cloud"), ("google.cloud", "bigquery"),
                          ("snowflake", "connector")):
        if parent in sys.modules and f"{parent}.{child}" in sys.modules:
            setattr(sys.modules[parent], child, sys.modules[f"{parent}.{child}"])


_stub_cloud_sdks()


def binary():
    return os.environ.get("SAYA_BENCH_BIN",
                          os.path.join(REPO, "target", "release", "saya"))


# ----------------------------------------------------------------- loading --

def _documents():
    cache = {}

    def read(name):
        if name not in cache:
            p = f"{CORPUS}/documents/{name}"
            cache[name] = open(p).read() if os.path.exists(p) else ""
        return cache[name]
    return read


def load_jobs(suites):
    """Every question, grouped into (suite, database, items) units of work.

    Grouping by database matters: questions for one database share a state
    store and run in sequence, which keeps schema-cache behaviour realistic
    while still letting different databases run in parallel.
    """
    doc, jobs = _documents(), []

    if "v1" in suites and os.path.exists(f"{CORPUS}/spider_data/dev.json"):
        by = {}
        for i, it in enumerate(json.load(open(f"{CORPUS}/spider_data/dev.json"))):
            it["_key"] = f"v1:{i:05d}"
            by.setdefault(it["db_id"], []).append(it)
        jobs += [("v1", db, qs) for db, qs in sorted(by.items())]

    lite = f"{CORPUS}/spider2-lite.jsonl"
    if not os.path.exists(lite):
        return jobs
    s2 = [json.loads(l) for l in open(lite) if l.strip()]

    if "v2local" in suites:
        by = {}
        for it in s2:
            if not it["instance_id"].startswith("local"):
                continue
            path = f"{CORPUS}/s2local/{it['db']}.sqlite"
            if not os.path.exists(path):
                continue
            it["_key"] = f"v2local:{it['instance_id']}"
            it["_path"] = path
            it["_doc"] = doc(it["external_knowledge"]) if it.get("external_knowledge") else ""
            by.setdefault(it["db"], []).append(it)
        jobs += [("v2local", db, qs) for db, qs in sorted(by.items())]

    if "v2bq" in suites and os.path.exists(f"{CORPUS}/bigquery_plan.json"):
        plan = json.load(open(f"{CORPUS}/bigquery_plan.json"))
        skip = set(filter(None, os.environ.get("SAYA_BENCH_SKIP_DBS", "").split(",")))
        by = {}
        for it in s2:
            if it["instance_id"][:2] not in ("bq", "ga"):
                continue
            if it["db"] not in plan or it["db"] in skip:
                continue
            it["_key"] = f"v2bq:{it['instance_id']}"
            it["_doc"] = doc(it["external_knowledge"]) if it.get("external_knowledge") else ""
            by.setdefault(it["db"], []).append(it)
        jobs += [("v2bq", db, qs) for db, qs in sorted(by.items())]
    return jobs


# --------------------------------------------------------------- executing --

def parse_ndjson(text):
    """Completed statements in order, plus the one nominated as the answer."""
    completed, pending, designated, attempts, consensus = [], None, None, 0, None
    for line in text.split("\n"):
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            ev = json.loads(line)
        except Exception:
            continue
        kind, name = ev.get("event"), ev.get("name")
        if kind == "tool_requested" and name == "bounded_sql_query":
            pending, attempts = ev.get("detail"), attempts + 1
        elif kind == "tool_completed" and name == "bounded_sql_query":
            if pending and "completed" in (ev.get("summary") or ""):
                completed.append(pending)
            pending = None
        elif kind == "answer_designated":
            designated = ev.get("sql")
        elif kind == "consensus_decided":
            # With several attempts there is one `answer_designated` per
            # attempt, so the last one is whichever attempt finished last —
            # not the answer the attempts agreed on. When consensus ran, its
            # verdict is the answer, and `sql` is absent when they disagreed.
            consensus = ev
            designated = ev.get("sql")
    return completed, designated, attempts, consensus


def question_text(item):
    q = item["question"]
    if item.get("_doc"):
        q += ("\n\n--- Reference documentation supplied with this question ---\n"
              + item["_doc"])
    return q


def session_prompt(item):
    """The question as one stdin line for the session surface.

    The bare REPL reads input line by line, so a multi-line question would be
    read as several turns. Whitespace is folded to single spaces, which only
    the Spider 2.0 reference documents (55 questions) ever notice: the words
    survive, the layout does not. A leading `/` and an empty fold are refused
    rather than sent — the session surface would read the first as a slash
    command and the second as nothing at all, and both would come back as a
    row that looks like a model that gave up.
    """
    folded = " ".join(question_text(item).split())
    if folded.startswith("/"):
        raise ValueError(f"question {item.get('_key', '<unknown>')!r} starts "
                         "with '/' and cannot be piped to the session surface")
    if not folded:
        raise ValueError(f"question {item.get('_key', '<unknown>')!r} is empty")
    return folded


def session_has_turns(sessions_dir):
    """Does this unit's session directory hold a session with a recorded turn?

    The store saves a session after every completed turn, so a file with
    `turns` means a prior question really did complete here — the evidence
    `--continue` needs. A file without turns (an attempt that errored before
    anything completed) counts as no session: continuing it would be
    indistinguishable from starting fresh. Corrupt files are skipped the same
    way the store skips them, and `.tmp` sidecars are not sessions.
    """
    try:
        files = sorted((os.path.getmtime(os.path.join(sessions_dir, f)), f)
                       for f in os.listdir(sessions_dir) if f.endswith(".json"))
    except OSError:
        return False
    for _, name in reversed(files):        # newest first, like the store does
        try:
            with open(os.path.join(sessions_dir, name)) as f:
                return bool(json.load(f).get("turns"))
        except (OSError, ValueError):
            continue
    return False


_lock = threading.Lock()


def emit(record, results=None):
    with _lock:
        with open(results or RESULTS, "a") as f:
            f.write(json.dumps(record) + "\n")


def question_command(bin_path, prompt, config, primary, extras, candidates=1):
    """The independent arm's per-question command: one `saya ask` process,
    no conversation behind it. Order of the flags is pinned by test."""
    cmd = [bin_path, "ask", "--config", config[0], "--connections", config[1],
           "--profile", primary, "--non-interactive",
           "--approval-mode", "read-only", "--format", "ndjson"]
    if candidates > 1:
        cmd += ["--candidates", str(candidates)]
    for e in extras:
        cmd += ["--include-profile", e]
    cmd.append(prompt)
    return cmd


def session_command(bin_path, config, primary, extras, continued=False):
    """The session arm's per-question command: the bare REPL, one turn.

    `--continue` joins the most recent session in the unit's session
    directory — the conversation the previous question saved. It is passed
    only when a session with turns is known to exist: with none, the REPL
    exits with "requested session was not found". The REPL cannot prompt
    (piped stdin implies no terminal) and takes no subcommand, so unlike the
    independent arm there is no `--non-interactive` to pass.
    """
    cmd = [bin_path, "--config", config[0], "--connections", config[1],
           "--profile", primary, "--approval-mode", "read-only",
           "--format", "ndjson"]
    for e in extras:
        cmd += ["--include-profile", e]
    if continued:
        cmd.append("--continue")
    return cmd


def run_unit(job, done, timeout, plan, candidates=1, session=False,
             results=None):
    if session and results is None:
        raise ValueError("session rows need their own results file — passing "
                         "none would land them in the independent arm's file")
    suite, db, items = job
    todo = [it for it in items if it["_key"] not in done]
    if not todo:
        return 0
    work = os.path.join(OUT, f"{ARM}{'-session' if session else ''}",
                        suite, db)
    os.makedirs(work, exist_ok=True)
    env = dict(os.environ, SAYA_CONFIG_HOME=HOME,
               SAYA_STATE_DB=f"{work}/state.db",
               SAYA_SESSION_DIR=f"{work}/sessions")
    key_file = os.environ.get("SAYA_BQ_KEY_FILE")
    if key_file and os.path.exists(key_file):
        env["SAYA_BQ_KEY"] = open(key_file).read()

    entry = plan.get(db, {}) if suite == "v2bq" else {}
    primary = entry.get("primary", db)
    extras = entry.get("extras", [])

    # Point at the benchmark's own configuration explicitly rather than relying
    # on discovery. The corpus lives inside the repository, and saya walks up
    # for a project-level `.saya/` — which this repository has, and which would
    # otherwise win over SAYA_CONFIG_HOME and hide every generated profile.
    # SAYA_BENCH_CONFIG selects an alternative config file so an arm can vary a
    # model setting (sampling temperature, say) without disturbing the corpus.
    config = [os.environ.get("SAYA_BENCH_CONFIG", f"{HOME}/saya/config.toml"),
              f"{HOME}/saya/connections.toml"]

    # Continuation is evidence-based, asked fresh before every question: the
    # store saves a session after each completed turn, so `session_has_turns`
    # answers exactly what `--continue` will find when the process starts. An
    # errored turn saves a turns-less session and counts as no context, so a
    # stretch of failed turns never leads the next question to claim
    # continuity it would not have — and a completed turn that a row was
    # never emitted for (a crash between turn and row) still counts, because
    # it really is in the conversation.
    sessions_dir = f"{work}/sessions"
    for it in todo:
        if session:
            continued = session_has_turns(sessions_dir)
            cmd = session_command(binary(), config, primary, extras,
                                  continued=continued)
            stdin = session_prompt(it) + "\n"
        else:
            continued = False
            cmd = question_command(binary(), question_text(it), config,
                                   primary, extras, candidates)
            stdin = None
        started = time.time()
        # A model gateway occasionally returns an invalid response. Over a run
        # this long that will happen, and scoring it as a miss would blame the
        # agent for the transport. Retry only when nothing ran at all, so a
        # genuinely wrong answer never gets a second attempt. In session mode
        # an errored turn leaves nothing in the conversation, so a retry asks
        # the same question at the same point in it.
        for attempt in range(3):
            try:
                proc = subprocess.run(cmd, capture_output=True, text=True,
                                      env=env, cwd=CORPUS, timeout=timeout,
                                      input=stdin)
                text, timed_out = proc.stdout + proc.stderr, False
            except subprocess.TimeoutExpired as expired:
                # The partial output is on the exception, and discarding it made
                # every timeout unattributable: the raw stream was overwritten
                # with "" and the row then reported `n_queries: 0`, which reads
                # as "the agent ran nothing" when it had in fact been working
                # until the wall clock killed it. Keep what it managed to say.
                # `TimeoutExpired` carries the streams as BYTES even when the
                # call asked for text, so decode before joining — concatenating
                # them with a str raises, the exception escapes, and the
                # question is dropped from the results entirely.
                def _text(stream):
                    if stream is None:
                        return ""
                    return stream.decode("utf-8", "replace") if isinstance(stream, bytes) else stream

                text = _text(expired.stdout) + _text(expired.stderr)
                timed_out = True
            queries, designated, attempts, consensus = parse_ndjson(text)
            # A `--continue` spawn can still find no session — a race with
            # another harness, or a session file that went corrupt between the
            # probe and the spawn. Fall back to a fresh session rather than
            # burning the retry budget on it, and mark the row honestly.
            if (session and continued and attempt == 0 and not queries
                    and '"event":"' not in text
                    and "session was not found" in text):
                continued = False
                cmd = session_command(binary(), config, primary, extras,
                                      continued=False)
                continue
            if queries or '"event":"error"' not in text or attempt == 2:
                break
            time.sleep(5 * (attempt + 1))
        # Keep the raw stream. Without it a question that ran no SQL is a dead
        # end — you cannot tell a model that gave up from a database that
        # refused it, and that distinction decides whether a result is valid.
        with open(f"{work}/{it['_key'].split(':')[-1]}.ndjson", "w") as raw:
            raw.write(text)
        record = {"suite": suite, "db": db, "key": it["_key"],
                  "mode": "session" if session else "independent",
                  "seconds": round(time.time() - started, 1),
                  "timed_out": timed_out, "attempts": attempts,
                  "provider_retries": attempt, "n_queries": len(queries),
                  "queries": queries, "designated": designated,
                  "consensus": consensus,
                  "dbfile": it.get("_path") or (
                      f"{CORPUS}/spider_data/database/{db}/{db}.sqlite"
                      if suite == "v1" else None),
                  "had_doc": bool(it.get("_doc")), "candidates": candidates,
                  "item": {k: it[k] for k in ("question", "query", "instance_id")
                           if k in it}}
        if session:
            record["session_continued"] = continued
        emit(record, results)
    return len(todo)


def warn_sharing(config_path):
    """Session continuity rides on saya's privacy rule — say so when it is off.

    Turns that queried the database are fed back to the provider only when
    `ai.allow_data_sharing` is on (or the provider is local). With it off the
    session arm still runs, but what it measures is recall and the state
    store, not conversation — a difference that must be visible before a run
    costs hours, not after. Comment lines don't count: a commented-out
    setting is off, and warning on the commented form would be the same
    false negative the search exists to prevent.
    """
    try:
        text = open(config_path).read()
    except OSError:
        return
    live = re.sub(r"#.*", "", text)
    if not re.search(r"allow_data_sharing\s*=\s*true", live):
        print(f"  ! session arm: {config_path} does not set "
              "ai.allow_data_sharing = true — turns that queried the database "
              "will not be fed back, so this arm measures recall and the "
              "state store, not conversation continuity", flush=True)


def _check_session_questions(jobs):
    """Refuse a session run whose questions cannot be piped to the session
    surface. Caught before anything is spent: a ValueError raised mid-unit
    from a worker thread would otherwise surface only after every other unit
    in the pool had finished, and the offending question would crash the run
    again on every resume."""
    bad = []
    for _suite, _db, items in jobs:
        for it in items:
            try:
                session_prompt(it)
            except ValueError as error:
                bad.append(str(error))
    if bad:
        sys.exit("session mode cannot send these questions: " + "; ".join(bad))


def cmd_run(args):
    if getattr(args, "session", False) and args.candidates > 1:
        sys.exit("session mode runs one attempt per question — the session "
                 "surface has no multi-attempt orchestration, so --candidates "
                 "N>1 would record candidates it never ran. Use the default "
                 "arm for --candidates.")
    if ARM.endswith("-session"):
        sys.exit("SAYA_BENCH_ARM names ending in '-session' are reserved — "
                 "the session arm appends that suffix to the results file and "
                 "work directories, and an arm that carries it would collide "
                 "with the session arm of the same base name")
    if not os.environ.get("SAYA_AI_API_KEY"):
        sys.exit("SAYA_AI_API_KEY is not set")
    if not os.path.exists(binary()):
        sys.exit(f"no saya binary at {binary()} — cargo build --release")
    session = bool(getattr(args, "session", False))
    results = results_path(session)
    if session:
        warn_sharing(os.environ.get("SAYA_BENCH_CONFIG",
                                    f"{HOME}/saya/config.toml"))
    os.makedirs(OUT, exist_ok=True)
    suites = SUITES if args.suite == "all" else (args.suite,)
    plan = ({} if not os.path.exists(f"{CORPUS}/bigquery_plan.json")
            else json.load(open(f"{CORPUS}/bigquery_plan.json")))
    done = set()
    if os.path.exists(results):
        for line in open(results):
            try:
                done.add(json.loads(line)["key"])
            except Exception:
                pass
    jobs = load_jobs(suites)
    if args.limit_per_db:
        jobs = [(s, db, items[: args.limit_per_db]) for s, db, items in jobs]
    if session:
        _check_session_questions(jobs)
    total = sum(len(j[2]) for j in jobs)
    summary = f"{len(jobs)} database units, {total} questions, {len(done)} already done"
    if session:
        print(f"session arm — one conversation per database: {summary} -> {results}")
    else:
        print(summary)
    started = time.time()
    with ThreadPoolExecutor(max_workers=args.workers) as ex:
        futures = [ex.submit(run_unit, j, done, args.timeout, plan,
                             args.candidates, session, results)
                   for j in jobs]
        for i, fut in enumerate(futures, 1):
            ran = fut.result()
            print(f"  [{i}/{len(jobs)}] {jobs[i-1][0]:<8} {jobs[i-1][1][:26]:<27} "
                  f"ran {ran:>3}   {round((time.time()-started)/60):>3}m", flush=True)
    print("run complete —  next: bench.py score"
          + (" --session" if session else ""))


# ----------------------------------------------------------------- scoring --

def sqlite_rows(dbfile, sql, limit=5000):
    import sqlite3
    con = sqlite3.connect(f"file:{dbfile}?mode=ro", uri=True)
    con.text_factory = lambda b: b.decode("utf-8", "replace")
    try:
        return con.execute(sql).fetchmany(limit)
    finally:
        con.close()


def score_v1(item, queries, dbfile):
    """Spider 1.0 execution accuracy: result-set equality, order-sensitive only
    when the gold query is ordered."""
    if not queries or not queries[-1]:
        return False
    try:
        pred = sqlite_rows(dbfile, queries[-1])
        gold = sqlite_rows(dbfile, item["query"])
    except Exception:
        return False
    norm = lambda rs: [tuple(str(v) for v in r) for r in rs]
    p, g = norm(pred), norm(gold)
    return p == g if "order by" in item["query"].lower() else sorted(p) == sorted(g)


_token = {"value": None, "at": 0.0}


def bq_token():
    if _token["value"] and time.time() - _token["at"] < 1800:
        return _token["value"]
    tok = subprocess.run(["gcloud", "auth", "print-access-token"],
                         capture_output=True, text=True).stdout.strip()
    _token.update(value=tok, at=time.time())
    return tok


def bq_frame(sql):
    """Re-runs a statement and returns a typed DataFrame, or None.

    This is close to free: it is the identical statement saya just ran, so
    BigQuery serves it from cache. The typing is not optional — the REST API
    returns every cell as a string while gold is read from CSV and arrives
    typed, so without casting a correct numeric answer compares unequal to an
    identical gold value.
    """
    import pandas as pd
    project = os.environ.get("SAYA_BQ_PROJECT")
    if not project:
        return None
    body = json.dumps({"query": sql, "useLegacySql": False, "useQueryCache": True,
                       "timeoutMs": 180000, "maxResults": 20000,
                       "maximumBytesBilled": str(10 * 1024**3)}).encode()
    req = urllib.request.Request(
        f"https://bigquery.googleapis.com/bigquery/v2/projects/{project}/queries",
        data=body, headers={"Authorization": f"Bearer {bq_token()}",
                            "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=200) as r:
            d = json.load(r)
    except Exception:
        return None
    if "error" in d or not d.get("jobComplete"):
        return None
    fields = d.get("schema", {}).get("fields", [])
    if not fields:
        return None
    frame = pd.DataFrame([[c["v"] for c in row["f"]] for row in d.get("rows", [])],
                         columns=[f["name"] for f in fields])
    for f in fields:
        kind, name = f.get("type", "STRING").upper(), f["name"]
        if kind in ("INTEGER", "INT64"):
            frame[name] = pd.to_numeric(frame[name], errors="coerce").astype("Int64")
        elif kind in ("FLOAT", "FLOAT64", "NUMERIC", "BIGNUMERIC"):
            frame[name] = pd.to_numeric(frame[name], errors="coerce")
        elif kind in ("BOOLEAN", "BOOL"):
            frame[name] = frame[name].map({"true": True, "false": False})
    return frame


_gold_cache = {}


def gold_tables(instance_id):
    """Every accepted gold answer for this question. Spider 2.0 publishes
    several acceptable result tables per question as _a/_b/_c variants."""
    import pandas as pd
    if instance_id in _gold_cache:
        return _gold_cache[instance_id]
    os.makedirs(f"{CORPUS}/gold", exist_ok=True)
    out = []
    for suffix in ("", "_a", "_b", "_c", "_d", "_e"):
        name = f"{instance_id}{suffix}.csv"
        path = f"{CORPUS}/gold/{name}"
        if not os.path.exists(path):
            try:
                urllib.request.urlretrieve(f"{GOLD_URL}/{name}", path)
            except Exception:
                open(path, "w").close()          # cache the miss
        if os.path.getsize(path) > 0:
            try:
                out.append(pd.read_csv(path))
            except Exception:
                pass
    _gold_cache[instance_id] = out
    return out


def _eval_meta(instance_id):
    path = f"{CORPUS}/spider2lite_eval.jsonl"
    if not hasattr(_eval_meta, "cache"):
        _eval_meta.cache = {}
        if os.path.exists(path):
            for line in open(path):
                if line.strip():
                    row = json.loads(line)
                    _eval_meta.cache[row["instance_id"]] = row
    return _eval_meta.cache.get(instance_id, {})


def _compare(pred, instance_id):
    from evaluate_utils import compare_multi_pandas_table
    golds = gold_tables(instance_id)
    if pred is None or not golds:
        return False
    meta = _eval_meta(instance_id)
    cond = meta.get("condition_cols", []) or []
    if isinstance(cond, str):
        cond = json.loads(cond or "[]")
    ignore_order = str(meta.get("ignore_order", "True")).lower() == "true"
    try:
        return bool(compare_multi_pandas_table(pred, golds, cond, ignore_order))
    except Exception:
        return False


def score_v2local(item, queries, dbfile):
    import sqlite3, pandas as pd
    if not queries or not queries[-1]:
        return False
    try:
        con = sqlite3.connect(f"file:{dbfile}?mode=ro", uri=True)
        pred = pd.read_sql_query(queries[-1], con)
        con.close()
    except Exception:
        return False
    return _compare(pred, item["instance_id"])


def score_v2bq(item, queries, _dbfile):
    if not queries or not queries[-1]:
        return False
    return _compare(bq_frame(queries[-1]), item["instance_id"])


SCORERS = {"v1": score_v1, "v2local": score_v2local, "v2bq": score_v2bq}


def cmd_score(args):
    session = bool(getattr(args, "session", False))
    rows = [json.loads(l) for l in open(results_path(session)) if l.strip()]
    latest = {r["key"]: r for r in rows}       # a resumed run may repeat a key
    scored = []
    for r in latest.values():
        scorer = SCORERS[r["suite"]]
        try:
            last = bool(scorer(r["item"], r["queries"], r["dbfile"]))
        except Exception:
            last = False
        if r["designated"]:
            try:
                des = bool(scorer(r["item"], [r["designated"]], r["dbfile"]))
            except Exception:
                des = False
        else:
            des = last
        scored.append({**r, "match": last, "match_designated": des,
                       "designated_given": r["designated"] is not None})
    with open(scored_path(session), "w") as f:
        for s in scored:
            f.write(json.dumps(s) + "\n")
    print(f"scored {len(scored)} questions -> {scored_path(session)}")
    cmd_report(args)


def cmd_report(args):
    rows = [json.loads(l) for l in open(scored_path(bool(getattr(args, "session", False)))) if l.strip()]
    print(f"\n{'suite':<32}{'n':>6}{'last':>9}{'designated':>13}"
          f"{'nominated':>11}{'no SQL':>8}{'timeout':>9}{'median':>8}")
    for suite in SUITES:
        sub = [r for r in rows if r["suite"] == suite]
        if not sub:
            continue
        n = len(sub)
        secs = sorted(r["seconds"] for r in sub)
        print(f"{LABEL[suite]:<32}{n:>6}"
              f"{100*sum(r['match'] for r in sub)/n:>8.1f}%"
              f"{100*sum(r['match_designated'] for r in sub)/n:>12.1f}%"
              f"{sum(r['designated_given'] for r in sub):>11}"
              f"{sum(1 for r in sub if r['n_queries'] == 0):>8}"
              f"{sum(1 for r in sub if r['timed_out']):>9}"
              f"{secs[n//2]:>7.0f}s")
    s2 = [r for r in rows if r["suite"] in ("v2local", "v2bq")]
    if s2:
        n = len(s2)
        print(f"\nSpider 2.0-lite, the {n} questions run "
              f"(Snowflake's 207 are not part of this harness):")
        print(f"  last {100*sum(r['match'] for r in s2)/n:.1f}%   "
              f"designated {100*sum(r['match_designated'] for r in s2)/n:.1f}%")
        print("  a leaderboard submission needs all 547 — do not quote this as "
              "a Spider 2.0-lite score")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--suite", choices=("all",) + SUITES, default="all")
    r.add_argument("--workers", type=int, default=8)
    r.add_argument("--timeout", type=int, default=600)
    r.add_argument("--candidates", type=int, default=1,
                   help="answer with the best of N independent attempts; N>1 "
                        "costs roughly N times as much")
    r.add_argument("--limit-per-db", type=int, default=0,
                   help="cap questions per database — for a quick smoke run")
    r.add_argument("--session", action="store_true",
                   help="one conversation per database — questions continue "
                        "the session the previous question saved (writes "
                        "results-<arm>-session.ndjson)")
    r.set_defaults(fn=cmd_run)
    s = sub.add_parser("score")
    s.add_argument("--session", action="store_true",
                   help="score the session arm's results file")
    s.set_defaults(fn=cmd_score)
    p = sub.add_parser("report")
    p.add_argument("--session", action="store_true",
                   help="report from the session arm's scored file")
    p.set_defaults(fn=cmd_report)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
