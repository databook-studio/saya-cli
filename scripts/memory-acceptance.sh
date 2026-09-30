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
#   scripts/memory-acceptance.sh --self-test           # scorer + plumbing vs
#                                                      # synthetic captures,
#                                                      # no model/db
#
# Requires (normal mode): .saya/config.toml (the intended ai settings — staged
# into the trusted user layer; see below), .env.saya (saya reads the key
# itself — this script never touches its value), the databook-postgres
# container (under podman or docker), a built binary, and python3 (the scorer
# is python stdlib). Set SAYA_BIN to override; defaults to target/debug/saya.
#
# The real-model mode reaches the configured gateway through the USER config
# layer: `--config` names the project layer, and the project layer is
# untrusted — its security-critical settings (`ai.base_url`, `ai.api_key`,
# `ai.allow_data_sharing`, `run.read_only`) are reverted with a warning, which
# is how the first real runs died with HTTP 401 before scoring anything. The
# intended file is therefore copied to `$SAYA_CONFIG_HOME/saya/config.toml`
# (the user layer, which owns those settings) and never passed as `--config`.
#
# Before any run, a preflight trivial `ask` proves the gateway actually
# authenticates; a 401/auth/transport failure — or the ignored-settings
# warning — exits 2 instead of being scored as the model ignoring the claim.
#
# Each run appends a fresh nonce to its prompt (a caching gateway replays
# identical requests in ~0.1 s, which is not an independent sample) and times
# the call: a run under one second is flagged as a probable cache hit and is
# counted neither for nor against.
#
# Exit codes: 0 = the fact bound on every independent sample (or the
# self-test passed).
# 1 = at least one independent sample overrode / was inconclusive, or a
# self-test case failed.
# 2 = the environment is not ready (missing inputs, container down, preflight
# could not reach the gateway, ignored-settings warning, or every run was a
# probable cache hit so no independent sample exists).
set -uo pipefail

ROOT="$(git rev-parse --show-toplevel)" || {
  echo "✗ not inside a git worktree (the script reads its fixtures and .saya/ from the repo root)" >&2
  exit 2
}
cd "$ROOT" || exit 2

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

# -- Run plumbing helpers ------------------------------------------------------
#
# The real-model mode's decisions, kept as functions so the self-test can hold
# them without a gateway, a database, or a built binary.

# Signatures that mean "the environment failed, not the model": an
# unauthenticated or unreachable gateway (the typed provider errors) and the
# one-shot warning that the project layer tried to set security-critical
# settings (the user layer must own them — see the config staging below).
ENV_FAILURE_PATTERN='provider request failed|provider is not configured|warning: ignored [0-9]+ security-critical'

# Scans BOTH preflight streams: ndjson renders error events to stderr, but
# scanning both cannot miss whichever stream a format change moves them to.
environment_failure_detected() {  # <stderr-file> <stdout-file> → 0 = failure found
  local stream
  for stream in "$1" "$2"; do
    if [ -s "$stream" ] && grep -Eq "$ENV_FAILURE_PATTERN" "$stream"; then
      return 0
    fi
  done
  return 1
}

# The gateway caches identical requests and replays the first answer in a
# fraction of a second; a real model call never lands that fast. A run under
# one second is therefore flagged and excluded from the sample count.
is_probable_cache_hit() {  # <seconds> → 0 = probable cache hit
  case "${1:-}" in ''|*[!0-9.]*) return 1 ;; esac
  awk -v t="$1" 'BEGIN { exit ((t + 0) < 1.0) ? 0 : 1 }'
}

# A per-run nonce appended to the prompt: identical prompts are NOT
# independent samples against a caching gateway, and a fixed suffix would
# repeat across script invocations too. The index keeps the shape readable;
# the two RANDOM draws make repeats within and across script runs improbable.
new_run_nonce() {  # <run-index>
  printf 'run-%s-%s-%s' "$1" "$RANDOM" "$RANDOM"
}

# The containers may run under podman or docker; on this Mac `docker ps` does
# not see podman's containers, so try podman first and fall back to docker.
container_running() {
  local engine
  for engine in podman docker; do
    if command -v "$engine" >/dev/null 2>&1 \
      && "$engine" ps --format '{{.Names}}' 2>/dev/null | grep -q databook-postgres
    then
      return 0
    fi
  done
  return 1
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

# -- Plumbing self-test --------------------------------------------------------
#
# The scorer cases above prove the oracle; these prove the run plumbing's
# decisions: which captured stream is an environment failure rather than a
# model verdict, which wall time is a probable cache hit, and that per-run
# nonces differ (independent samples). All pure: synthetic captures, no model,
# gateway, database, or built binary.

PLUMB_SCRATCH=""

case_http_401_stderr_is_an_environment_failure() {
  printf 'provider request failed: HTTP 401: authentication failed\n' \
    > "$PLUMB_SCRATCH/err"
  : > "$PLUMB_SCRATCH/out"
  environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_a_benign_diagnostic_is_not_an_environment_failure() {
  printf '{"event":"diagnostic","message":"schema cache is warm"}\n' \
    > "$PLUMB_SCRATCH/err"
  printf '{"event":"assistant_text","text":"OK"}\n' > "$PLUMB_SCRATCH/out"
  ! environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_ignored_settings_warning_is_an_environment_failure() {
  printf '%s\n' \
    "warning: ignored 4 security-critical settings from this project's .saya/config.toml — run \`saya config doctor\` for which, and why." \
    > "$PLUMB_SCRATCH/err"
  : > "$PLUMB_SCRATCH/out"
  environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_provider_not_configured_is_an_environment_failure() {
  printf 'provider is not configured: no api_key reference\n' > "$PLUMB_SCRATCH/err"
  : > "$PLUMB_SCRATCH/out"
  environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_error_event_on_stdout_is_an_environment_failure() {
  : > "$PLUMB_SCRATCH/err"
  printf '%s\n' \
    '{"event":"error","message":"provider request failed: HTTP 401: authentication failed"}' \
    > "$PLUMB_SCRATCH/out"
  environment_failure_detected "$PLUMB_SCRATCH/err" "$PLUMB_SCRATCH/out"
}

case_cache_threshold_classifies_wall_times() {
  is_probable_cache_hit 0.42 || return 1
  is_probable_cache_hit 0.99 || return 1
  if is_probable_cache_hit 1.00; then return 1; fi
  if is_probable_cache_hit 12.34; then return 1; fi
}

case_run_nonces_differ_and_match_their_shape() {
  local one two
  one="$(new_run_nonce 1)"
  two="$(new_run_nonce 1)"
  # The property that defeats the cache: two draws for the SAME index differ,
  # so prompts never repeat across script invocations.
  [ -n "$one" ] && [ -n "$two" ] && [ "$one" != "$two" ] || return 1
  printf '%s\n%s\n' "$one" "$two" | grep -Eq '^run-[0-9]+-[0-9]+-[0-9]+$'
}

plumbing_test() {
  local scratch fails=0 total=0 idx
  scratch="$(mktemp -d)" \
    || { echo "✗ plumbing self-test could not create a scratch dir" >&2; return 2; }
  PLUMB_SCRATCH="$scratch"

  local -a names labels
  names=(
    case_http_401_stderr_is_an_environment_failure
    case_a_benign_diagnostic_is_not_an_environment_failure
    case_ignored_settings_warning_is_an_environment_failure
    case_provider_not_configured_is_an_environment_failure
    case_error_event_on_stdout_is_an_environment_failure
    case_cache_threshold_classifies_wall_times
    case_run_nonces_differ_and_match_their_shape
  )
  labels=(
    "HTTP 401 on the preflight stderr is an environment failure"
    "a benign diagnostic is not an environment failure"
    "the ignored security-critical settings warning is an environment failure"
    "a not-configured provider is an environment failure"
    "an error event on the preflight stdout is an environment failure"
    "cache-hit threshold: 0.42 yes, 1.00 no, 12.34 no"
    "run nonces: same-index draws differ, shape run-<i>-<r>-<r>"
  )
  for idx in "${!names[@]}"; do
    total=$((total + 1))
    if "${names[$idx]}"; then
      printf 'ok  plumbing: %s\n' "${labels[$idx]}"
    else
      printf 'FAIL plumbing: %s\n' "${labels[$idx]}"
      fails=$((fails + 1))
    fi
  done
  rm -rf "$scratch"
  PLUMB_SCRATCH=""
  echo
  if [ "$fails" -eq 0 ]; then
    printf '✓ %d/%d plumbing cases pass\n' "$total" "$total"
    return 0
  fi
  printf '✗ %d/%d plumbing cases fail\n' "$fails" "$total"
  return 1
}

if [ "${1:-}" = "--self-test" ]; then
  self_test
  fixture_status=$?
  plumbing_test
  plumbing_status=$?
  if [ "$fixture_status" -ne 0 ]; then exit "$fixture_status"; fi
  exit "$plumbing_status"
fi

RUNS="${1:-3}"
case "$RUNS" in
  ''|*[!0-9]*) { echo "✗ runs-per-prompt must be a number (got: $RUNS)" >&2; exit 2; } ;;
esac
if [ "$RUNS" -lt 1 ]; then
  echo "✗ runs-per-prompt must be at least 1 (a zero-run experiment passes vacuously and proves nothing)" >&2
  exit 2
fi
BIN="${SAYA_BIN:-target/debug/saya}"
OBJECT="pagila.public.rental"

[ -x "$BIN" ] || { echo "✗ $BIN not built (cargo build -p saya-cli)" >&2; exit 2; }
[ -s .env.saya ] || { echo "✗ .env.saya missing or empty" >&2; exit 2; }
[ -s .saya/config.toml ] \
  || { echo "✗ .saya/config.toml missing or empty (the intended ai settings live there)" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "✗ python3 not found (the scorer needs it)" >&2; exit 2; }
container_running \
  || { echo "✗ databook-postgres is not running (checked podman, then docker)" >&2; exit 2; }

# An isolated HOME so the acceptance run never reads or writes the developer's
# real contract store. Rebuilt each run, so the result depends only on the fact
# this script establishes.
HOME_DIR="$(mktemp -d)"
STREAM="$(mktemp)"; STDERRS="$(mktemp)"; REASON="$(mktemp)"
PREFLIGHT_OUT="$(mktemp)"; PREFLIGHT_ERR="$(mktemp)"
trap 'rm -rf "$HOME_DIR" "$STREAM" "$STDERRS" "$REASON" "$PREFLIGHT_OUT" "$PREFLIGHT_ERR"' EXIT
export HOME="$HOME_DIR"
export SAYA_DOCKER_POSTGRES_PASSWORD=databook SAYA_DOCKER_MYSQL_PASSWORD=databook

# The intended ai settings live in the project layer, which saya does not
# trust: passed as `--config` (or discovered from the cwd), their security-
# critical fields are reverted with a warning. They belong in the USER layer,
# which owns them — staged under SAYA_CONFIG_HOME, the user config dir. The
# preflight below refuses to run if the warning still appears.
export SAYA_CONFIG_HOME="$HOME_DIR/config"
mkdir -p "$SAYA_CONFIG_HOME/saya" \
  || { echo "✗ could not create the user config dir" >&2; exit 2; }
cp .saya/config.toml "$SAYA_CONFIG_HOME/saya/config.toml" \
  || { echo "✗ could not stage .saya/config.toml into the user layer" >&2; exit 2; }

SAYA=("$BIN" --connections .saya/connections.toml --profile docker_postgres)

# The preflight proves the run can actually reach the configured gateway: one
# trivial ask. `config doctor` cannot prove this — it performs no network
# checks — so the proof is a real request. An environment failure here exits 2
# before anything is scored; that is the difference between "the model ignored
# the claim" and "the environment was never ready".
"${SAYA[@]}" --env-file .env.saya --approval-mode read-only --non-interactive \
  --format ndjson ask "Connectivity check: reply with the single word OK and nothing else." \
  >"$PREFLIGHT_OUT" 2>"$PREFLIGHT_ERR"
preflight_status=$?
if environment_failure_detected "$PREFLIGHT_ERR" "$PREFLIGHT_OUT"; then
  detail="$(grep -Ehm1 "$ENV_FAILURE_PATTERN" "$PREFLIGHT_ERR" "$PREFLIGHT_OUT" 2>/dev/null \
    | cut -c1-160 | sed -n '1p')"
  echo "✗ environment not ready: ${detail:-the preflight request failed}" >&2
  exit 2
fi
if [ "$preflight_status" -ne 0 ]; then
  echo "✗ environment not ready: the preflight ask exited $preflight_status" >&2
  [ -s "$PREFLIGHT_ERR" ] \
    && tail -n 3 "$PREFLIGHT_ERR" | cut -c1-160 | sed 's/^/     /' >&2
  exit 2
fi

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

# Per run: a fresh nonce defeats the gateway's request cache, and the wall
# time tells an independent call from a replayed one. A probable cache hit is
# reported but counted neither for nor against — it is not an independent
# sample, so it cannot prove or break the claim.
samples=0
sample_failures=0
cache_hits=0
for prompt in "${PROMPTS[@]}"; do
  echo "── \"$prompt\""
  for i in $(seq 1 "$RUNS"); do
    : > "$STREAM"; : > "$STDERRS"; : > "$REASON"
    nonce="$(new_run_nonce "$i")"
    started="$(python3 -c 'import time; print(time.monotonic())')"
    "${SAYA[@]}" --env-file .env.saya --approval-mode read-only --non-interactive \
      --format ndjson ask "$prompt ($nonce)" >"$STREAM" 2>"$STDERRS"
    ended="$(python3 -c 'import time; print(time.monotonic())')"
    elapsed="$(awk -v a="$started" -v b="$ended" 'BEGIN{printf "%.2f", b - a}')"
    # The verdict comes only from the structured stream: executed SQL with a
    # successful completion. Prose, denials, and failed executions never score.
    verdict="$(score_stream "$STREAM" 2>"$REASON")"
    reason="$(cat "$REASON")"
    if is_probable_cache_hit "$elapsed"; then
      cache_hits=$((cache_hits + 1))
      cached=yes
      timing="[$elapsed s — probable cache hit, not counted as a sample]"
    else
      samples=$((samples + 1))
      cached=no
      timing="[$elapsed s]"
    fi
    case "$verdict" in
      BOUND) echo "   run $i: bound     ($CLAIMED) $timing" ;;
      OVERRIDDEN)
        echo "   run $i: OVERRIDDEN ($GUESS) $timing"
        # A cache-hit run is not an independent sample: neither evidence for
        # nor against.
        [ "$cached" = no ] && sample_failures=$((sample_failures + 1))
        ;;
      *)
        echo "   run $i: inconclusive — ${reason:-no verdict} $timing"
        if [ -s "$STDERRS" ]; then
          tail -n 1 "$STDERRS" | cut -c1-160 | sed 's/^/     stderr: /'
        fi
        [ "$cached" = no ] && sample_failures=$((sample_failures + 1))
        ;;
    esac
  done
done

echo
if [ "$samples" -eq 0 ] && [ "$cache_hits" -gt 0 ]; then
  echo "✗ environment not ready: all $cache_hits run(s) were probable cache hits (under 1 s);" >&2
  echo "  no independent sample reached the model — the gateway may be replaying identical requests." >&2
  exit 2
fi
if [ "$cache_hits" -gt 0 ]; then
  excluded=" ($cache_hits probable cache hit(s) excluded from scoring)"
else
  excluded=""
fi
if [ "$sample_failures" -eq 0 ]; then
  echo "✓ the confirmed claim bound on all $samples independent sample(s)$excluded"
  exit 0
fi
echo "✗ $sample_failures of $samples independent sample(s) did not honour the confirmed claim$excluded"
exit 1