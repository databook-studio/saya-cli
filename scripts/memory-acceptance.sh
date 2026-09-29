#!/usr/bin/env bash
# Acceptance test: does a remembered fact actually change the SQL saya generates?
#
# This is the gate the assisted-memory work must pass before any demo is recorded.
# It is deliberately empirical: it asks the real model, through the real gateway,
# against the real pagila database, and reports the column each answer actually
# filtered on. It never inspects a prompt or a mock.
#
# The oracle reads the structured `--format ndjson` stream of each run and scores
# only the SQL of queries that *executed successfully* (a tool_completed success
# paired with its tool_requested SQL). Prose never scores: a sentence naming a
# column is not evidence the query used it. No successful query is INCONCLUSIVE
# and fails closed.
#
# Usage:
#   scripts/memory-acceptance.sh [runs-per-prompt]     # default 3
#   scripts/memory-acceptance.sh --self-test           # scorer vs fixtures, no model/db
#
# Requires (normal mode): .env.saya (saya reads the key itself — this script
# never touches its value), the databook-postgres container, a built binary, and
# python3 (the scorer is python stdlib). Set SAYA_BIN to override; defaults to
# target/debug/saya.
#
# Exit codes: 0 = the fact bound on every run (or the self-test passed).
# 1 = at least one override / inconclusive run / failing self-test case.
# 2 = the environment is not ready.
set -uo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

CLAIMED="return_date"      # the business rule the user established
GUESS="rental_date"        # what the model picks unaided, without the contract
FIXTURES="scripts/testdata/memory-acceptance"

# The verdict oracle. Reads a capture of one run's ndjson stdout (the structured
# events `saya ask --format ndjson` prints) and prints BOUND, OVERRIDDEN or
# INCONCLUSIVE on stdout; a human-readable reason goes to stderr.
#
# What scores, per the audit (TA-07): only the SQL of queries that executed
# successfully. The wire carries the SQL on `tool_requested` events (the
# `detail` field: collapsed SQL, optionally suffixed ` (@connection)`), and the
# success witness on the matching `tool_completed` event — `bounded_sql_query`
# completes as "read-only database tool completed" and fails with a summary
# keeping the substring "failed" (denied calls emit `tool_denied` and never
# complete). Each completion is paired with the oldest unanswered request of
# the same tool, so a failed or denied execution leaves its SQL unwitnessed and
# prose can never score. `answer_designated` / `consensus_decided` name the
# answering query; when their SQL is in the witnessed set, it alone decides.
# python3 stdlib only, by design: no jq dependency, no grep over prose.
score_stream() {
  python3 - "$1" "$CLAIMED" "$GUESS" <<'PYSCORER'
import json
import re
import sys

# Every tool whose `sql` argument really executes against the database, i.e.
# whose request detail carries the executed statement (tool_call_detail's set).
SQL_TOOLS = frozenset(
    {
        "bounded_sql_query",
        "bounded_sql_query_all",
        "render_chart",
        "result_shape",
        "column_health",
        "join_check",
    }
)
# Where a WHERE-clause slice ends: the next clause keyword. Subquery WHEREs
# get their own slice, so a JOIN's ON columns stay out of the predicate text.
CLAUSE_KEYWORDS = re.compile(
    r"\b(?:GROUP\s+BY|ORDER\s+BY|HAVING|LIMIT|OFFSET|WINDOW|UNION|INTERSECT"
    r"|EXCEPT|FETCH\s+(?:FIRST|NEXT)|FOR\s+UPDATE)\b",
    re.IGNORECASE,
)
WHERE_KEYWORD = re.compile(r"\bWHERE\b", re.IGNORECASE)
SINGLE_QUOTED = re.compile(r"'(?:[^']|'')*'")
TRAILING_TARGET = re.compile(r"\s+\((?:@[^()\s]*|all connected databases)\)$")

BOUND, OVERRIDDEN, INCONCLUSIVE = "BOUND", "OVERRIDDEN", "INCONCLUSIVE"


def norm(sql):
    """One normalized form for identity: collapse whitespace runs, trim."""
    return re.sub(r"\s+", " ", sql).strip()


def sql_from_detail(detail):
    """The requested SQL as the wire's `detail` field carries it."""
    sql = TRAILING_TARGET.sub("", detail).strip()
    return norm(sql)


def where_slices(sql):
    for match in WHERE_KEYWORD.finditer(sql):
        tail = sql[match.end() :]
        cut = CLAUSE_KEYWORDS.search(tail)
        yield tail[: cut.start()] if cut else tail


def classify(sql, claimed, guess):
    """Which time column one statement predicates on, from its WHERE slices."""
    text = " ".join(SINGLE_QUOTED.sub("''", w) for w in where_slices(sql))
    has = lambda column: re.search(rf"\b{re.escape(column)}\b", text) is not None
    claimed_hit, guess_hit = has(claimed), has(guess)
    if claimed_hit and guess_hit:
        return "AMBIGUOUS"
    if claimed_hit:
        return BOUND
    if guess_hit:
        return OVERRIDDEN
    return "NO_PREDICATE"


def witness_sqls(path, claimed, guess):
    """SQL of successfully executed queries, in stream order."""
    witnessed, pending = [], {}
    designated = consensus = None
    with open(path, encoding="utf-8", errors="replace") as stream:
        for raw in stream:
            line = raw.strip()
            if not line:
                continue
            try:
                event = json.loads(line)
            except ValueError:
                continue  # a non-JSON line is not evidence of anything
            if not isinstance(event, dict):
                continue
            tag, name = event.get("event"), event.get("name")
            if tag == "tool_requested" and name in SQL_TOOLS:
                detail = event.get("detail")
                sql = sql_from_detail(detail) if isinstance(detail, str) else None
                if sql:
                    pending.setdefault(name, []).append(sql)
            elif tag == "tool_completed" and name in SQL_TOOLS and isinstance(
                event.get("summary"), str
            ):
                queue = pending.get(name)
                sql = queue.pop(0) if queue else None
                summary = event["summary"]
                # A positive execution witness only: "failed" (the failure
                # summary keeps the substring) and "denied" never witness.
                if sql and "failed" not in summary and "denied" not in summary:
                    witnessed.append(sql)
            elif tag == "answer_designated" and isinstance(event.get("sql"), str):
                designated = norm(event["sql"])
            elif tag == "consensus_decided" and isinstance(event.get("sql"), str):
                consensus = norm(event["sql"])
    # The answering query decides when the stream named one and it executed;
    # otherwise every distinct successful statement must agree to speak.
    if designated is not None and designated in witnessed:
        return [designated]
    if consensus is not None and consensus in witnessed:
        return [consensus]
    unique = []
    for sql in witnessed:
        if sql not in unique:
            unique.append(sql)
    return unique


def verdict_for(path, claimed, guess):
    picks = witness_sqls(path, claimed, guess)
    if not picks:
        return INCONCLUSIVE, "no successful query executed"
    kinds = {classify(sql, claimed, guess) for sql in picks}
    if kinds == {BOUND}:
        return BOUND, f"the executed query filtered on {claimed}"
    if kinds == {OVERRIDDEN}:
        return OVERRIDDEN, f"the executed query filtered on {guess}"
    if "AMBIGUOUS" in kinds:
        return INCONCLUSIVE, "the executed query's WHERE names both time columns"
    if kinds == {"NO_PREDICATE"}:
        return INCONCLUSIVE, "the executed query carried no time predicate"
    return (
        INCONCLUSIVE,
        "successful queries disagree on the time column and none was designated",
    )


verdict, reason = verdict_for(sys.argv[1], sys.argv[2], sys.argv[3])
print(verdict)
print(reason, file=sys.stderr)
PYSCORER
}

# --self-test: run the scorer over the fixture streams and compare each against
# the verdict encoded in its filename (`expected-<verdict>--<slug>.ndjson`).
# Needs no model, gateway, database, or built binary.
self_test() {
  local fails=0 total=0 f name expected verdict
  for f in "$FIXTURES"/expected-*.ndjson; do
    if [ ! -e "$f" ]; then
      echo "✗ no fixtures found in $FIXTURES" >&2
      return 2
    fi
    name="$(basename "$f")"
    expected="${name#expected-}"
    expected="${expected%%--*}"
    expected="$(printf '%s' "$expected" | tr '[:lower:]' '[:upper:]')"
    total=$((total + 1))
    case "$expected" in
      BOUND|OVERRIDDEN|INCONCLUSIVE) ;;
      *)
        printf 'FAIL %s — filename encodes an unknown verdict: %s\n' "$name" "$expected"
        fails=$((fails + 1))
        continue
        ;;
    esac
    verdict="$(score_stream "$f" 2>/dev/null)"
    if [ "$verdict" = "$expected" ]; then
      printf 'ok  %s → %s\n' "$name" "$verdict"
    else
      printf 'FAIL %s → got %s, expected %s\n' "$name" "$verdict" "$expected"
      fails=$((fails + 1))
    fi
  done
  echo
  if [ "$fails" -eq 0 ]; then
    printf '✓ %d/%d self-test cases pass\n' "$total" "$total"
    return 0
  fi
  printf '✗ %d/%d self-test cases fail\n' "$fails" "$total"
  return 1
}

if [ "${1:-}" = "--self-test" ]; then
  self_test
  exit $?
fi

RUNS="${1:-3}"
case "$RUNS" in
  ''|*[!0-9]*) { echo "✗ runs-per-prompt must be a number (got: $RUNS)" >&2; exit 2; } ;;
esac
BIN="${SAYA_BIN:-target/debug/saya}"
OBJECT="pagila.public.rental"

[ -x "$BIN" ] || { echo "✗ $BIN not built (cargo build -p saya-cli)" >&2; exit 2; }
[ -s .env.saya ] || { echo "✗ .env.saya missing or empty" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "✗ python3 not found (the scorer needs it)" >&2; exit 2; }
docker ps --format '{{.Names}}' | grep -q databook-postgres \
  || { echo "✗ databook-postgres is not running" >&2; exit 2; }

# An isolated HOME so the acceptance run never reads or writes the developer's
# real contract store. Rebuilt each run, so the result depends only on the fact
# this script establishes.
HOME_DIR="$(mktemp -d)"
STREAM="$(mktemp)"; STDERRS="$(mktemp)"; REASON="$(mktemp)"
trap 'rm -rf "$HOME_DIR" "$STREAM" "$STDERRS" "$REASON"' EXIT
export HOME="$HOME_DIR"
export SAYA_DOCKER_POSTGRES_PASSWORD=databook SAYA_DOCKER_MYSQL_PASSWORD=databook

SAYA=("$BIN" --config .saya/config.toml --connections .saya/connections.toml --profile docker_postgres)

# The schema must be cached before the claim is made, or the claim is fingerprinted
# against nothing and lands `needs_review` instead of `current`.
"${SAYA[@]}" connection schema docker_postgres --refresh >/dev/null 2>&1 \
  || { echo "✗ schema refresh failed" >&2; exit 2; }
"${SAYA[@]}" contracts remember "$OBJECT" --kind time-column --value "$CLAIMED" >/dev/null \
  || { echo "✗ could not remember the claim" >&2; exit 2; }

# Two phrasings of one question. The second differs only by a trailing sentence,
# and that alone was enough to make the model discard a confirmed claim.
PROMPTS=(
  "How many rentals were there in each month of 2022?"
  "How many rentals were there in each month of 2022? Show me the SQL."
)

echo "claim: $OBJECT default_time_column = $CLAIMED  (confirmed, user_explicit)"
echo "runs per prompt: $RUNS"
echo

failures=0
for prompt in "${PROMPTS[@]}"; do
  echo "── \"$prompt\""
  for i in $(seq 1 "$RUNS"); do
    : > "$STREAM"; : > "$STDERRS"; : > "$REASON"
    "${SAYA[@]}" --env-file .env.saya --approval-mode read-only --non-interactive \
      --format ndjson ask "$prompt" >"$STREAM" 2>"$STDERRS"
    # The verdict comes only from the structured stream: executed SQL with a
    # successful completion. Prose, denials, and failed executions never score.
    verdict="$(score_stream "$STREAM" 2>"$REASON")"
    reason="$(cat "$REASON")"
    case "$verdict" in
      BOUND) echo "   run $i: bound     ($CLAIMED)" ;;
      OVERRIDDEN)
        echo "   run $i: OVERRIDDEN ($GUESS)"
        failures=$((failures + 1))
        ;;
      *)
        echo "   run $i: inconclusive — ${reason:-no verdict}"
        if [ -s "$STDERRS" ]; then
          tail -n 1 "$STDERRS" | cut -c1-160 | sed 's/^/     stderr: /'
        fi
        failures=$((failures + 1))
        ;;
    esac
  done
done

echo
if [ "$failures" -eq 0 ]; then
  echo "✓ the confirmed claim bound on every run"
  exit 0
fi
echo "✗ $failures run(s) did not honour the confirmed claim"
exit 1