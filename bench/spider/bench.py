#!/usr/bin/env python3
"""Runs and scores saya against Spider 1.0 and Spider 2.0-lite.

    python bench/spider/bench.py run    [--suite all] [--workers 8]
    python bench/spider/bench.py score
    python bench/spider/bench.py report

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

Environment:

    SAYA_AI_API_KEY   required — the model API key
    SAYA_BQ_KEY_FILE  path to a BigQuery service-account JSON (v2bq only)
    SAYA_BQ_PROJECT   the project that runs and is billed for the jobs
    SAYA_BENCH_BIN    saya binary (default: target/release/saya)
"""
import argparse, json, os, subprocess, sys, threading, time, urllib.request
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(ROOT, "..", ".."))
CORPUS = os.path.join(REPO, ".bench", "spider")
OUT = os.path.join(CORPUS, "results")
RESULTS = os.path.join(OUT, "results.ndjson")
SCORED = os.path.join(OUT, "scored.ndjson")
HOME = os.path.join(CORPUS, "home")
GOLD_URL = ("https://raw.githubusercontent.com/xlang-ai/Spider2/main/"
            "spider2-lite/evaluation_suite/gold/exec_result")

SUITES = ("v1", "v2local", "v2bq")
LABEL = {"v1": "Spider 1.0 dev (SQLite)",
         "v2local": "Spider 2.0-lite local (SQLite)",
         "v2bq": "Spider 2.0-lite (BigQuery)"}

sys.path.insert(0, CORPUS)          # evaluate_utils.py is fetched by setup.py


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
    completed, pending, designated, attempts = [], None, None, 0
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
    return completed, designated, attempts


def question_text(item):
    q = item["question"]
    if item.get("_doc"):
        q += ("\n\n--- Reference documentation supplied with this question ---\n"
              + item["_doc"])
    return q


_lock = threading.Lock()


def emit(record):
    with _lock:
        with open(RESULTS, "a") as f:
            f.write(json.dumps(record) + "\n")


def run_unit(job, done, timeout, plan):
    suite, db, items = job
    todo = [it for it in items if it["_key"] not in done]
    if not todo:
        return 0
    work = os.path.join(OUT, suite, db)
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
    config = [f"{HOME}/saya/config.toml", f"{HOME}/saya/connections.toml"]

    for it in todo:
        cmd = [binary(), "ask", "--config", config[0], "--connections", config[1],
               "--profile", primary, "--non-interactive",
               "--approval-mode", "read-only", "--format", "ndjson"]
        for e in extras:
            cmd += ["--include-profile", e]
        cmd.append(question_text(it))
        started = time.time()
        # A model gateway occasionally returns an invalid response. Over a run
        # this long that will happen, and scoring it as a miss would blame the
        # agent for the transport. Retry only when nothing ran at all, so a
        # genuinely wrong answer never gets a second attempt.
        for attempt in range(3):
            try:
                proc = subprocess.run(cmd, capture_output=True, text=True,
                                      env=env, cwd=CORPUS, timeout=timeout)
                text, timed_out = proc.stdout + proc.stderr, False
            except subprocess.TimeoutExpired:
                text, timed_out = "", True
            queries, designated, attempts = parse_ndjson(text)
            if queries or '"event":"error"' not in text or attempt == 2:
                break
            time.sleep(5 * (attempt + 1))
        # Keep the raw stream. Without it a question that ran no SQL is a dead
        # end — you cannot tell a model that gave up from a database that
        # refused it, and that distinction decides whether a result is valid.
        with open(f"{work}/{it['_key'].split(':')[-1]}.ndjson", "w") as raw:
            raw.write(text)
        emit({"suite": suite, "db": db, "key": it["_key"],
              "seconds": round(time.time() - started, 1),
              "timed_out": timed_out, "attempts": attempts,
              "provider_retries": attempt, "n_queries": len(queries),
              "queries": queries, "designated": designated,
              "dbfile": it.get("_path") or (
                  f"{CORPUS}/spider_data/database/{db}/{db}.sqlite"
                  if suite == "v1" else None),
              "had_doc": bool(it.get("_doc")),
              "item": {k: it[k] for k in ("question", "query", "instance_id")
                       if k in it}})
    return len(todo)


def cmd_run(args):
    if not os.environ.get("SAYA_AI_API_KEY"):
        sys.exit("SAYA_AI_API_KEY is not set")
    if not os.path.exists(binary()):
        sys.exit(f"no saya binary at {binary()} — cargo build --release")
    os.makedirs(OUT, exist_ok=True)
    suites = SUITES if args.suite == "all" else (args.suite,)
    plan = ({} if not os.path.exists(f"{CORPUS}/bigquery_plan.json")
            else json.load(open(f"{CORPUS}/bigquery_plan.json")))
    done = set()
    if os.path.exists(RESULTS):
        for line in open(RESULTS):
            try:
                done.add(json.loads(line)["key"])
            except Exception:
                pass
    jobs = load_jobs(suites)
    if args.limit_per_db:
        jobs = [(s, db, items[: args.limit_per_db]) for s, db, items in jobs]
    total = sum(len(j[2]) for j in jobs)
    print(f"{len(jobs)} database units, {total} questions, {len(done)} already done")
    started = time.time()
    with ThreadPoolExecutor(max_workers=args.workers) as ex:
        futures = [ex.submit(run_unit, j, done, args.timeout, plan) for j in jobs]
        for i, fut in enumerate(futures, 1):
            ran = fut.result()
            print(f"  [{i}/{len(jobs)}] {jobs[i-1][0]:<8} {jobs[i-1][1][:26]:<27} "
                  f"ran {ran:>3}   {round((time.time()-started)/60):>3}m", flush=True)
    print("run complete —  next: bench.py score")


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
    rows = [json.loads(l) for l in open(RESULTS) if l.strip()]
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
    with open(SCORED, "w") as f:
        for s in scored:
            f.write(json.dumps(s) + "\n")
    print(f"scored {len(scored)} questions -> {SCORED}")
    cmd_report(args)


def cmd_report(_args):
    rows = [json.loads(l) for l in open(SCORED) if l.strip()]
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
    r.add_argument("--limit-per-db", type=int, default=0,
                   help="cap questions per database — for a quick smoke run")
    r.set_defaults(fn=cmd_run)
    sub.add_parser("score").set_defaults(fn=cmd_score)
    sub.add_parser("report").set_defaults(fn=cmd_report)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
