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
# get their own slice.
CLAUSE_KEYWORDS = re.compile(
    r"\b(?:GROUP\s+BY|ORDER\s+BY|HAVING|LIMIT|OFFSET|WINDOW|UNION|INTERSECT"
    r"|EXCEPT|FETCH\s+(?:FIRST|NEXT)|FOR\s+UPDATE)\b",
    re.IGNORECASE,
)
WHERE_KEYWORD = re.compile(r"\bWHERE\b", re.IGNORECASE)
ON_KEYWORD = re.compile(r"\bON\b", re.IGNORECASE)
# Where an ON clause ends: the next join's start (spelled so a LEFT(…) function
# call never terminates it), a WHERE, or another clause keyword.
JOIN_KEYWORD = re.compile(
    r"\b(?:NATURAL\s+)?(?:(?:INNER|LEFT|RIGHT|FULL|CROSS)(?:\s+OUTER)?\s+)?JOIN\b",
    re.IGNORECASE,
)
CONJUNCT_KEYWORD = re.compile(r"\b(?:AND|OR)\b", re.IGNORECASE)
# Comparison operators that are NOT ranges (masked before the range check so a
# join key like ON a.x <> b.y is never read as one).
INEQUALITY_OP = re.compile(r"<>|!=")
# A range/order comparison; BETWEEN is matched by keyword, >=/<= before >/<.
RANGE_OP = re.compile(r">=|<=|>|<|\bBETWEEN\b")
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


def on_range_conjuncts(sql):
    """ON-clause conjuncts that compare something with a range op.

    A join's equality keys (`ON r.customer_id = c.customer_id`, even
    `ON a.return_date = b.return_date`) never count: the claimed or guessed
    column must sit in a >=, >, <=, <, or BETWEEN comparison for the ON clause
    to speak about time. Only such conjuncts are yielded, so an ON mention
    inside an equality key can never score.
    """
    for match in ON_KEYWORD.finditer(sql):
        tail = sql[match.end() :]
        ends = [
            cut.start()
            for cut in (
                CLAUSE_KEYWORDS.search(tail),
                WHERE_KEYWORD.search(tail),
                JOIN_KEYWORD.search(tail),
            )
            if cut
        ]
        clause = tail[: min(ends)] if ends else tail
        masked = SINGLE_QUOTED.sub("''", clause)
        for conjunct in CONJUNCT_KEYWORD.split(masked):
            comparable = INEQUALITY_OP.sub(" ", conjunct)
            if RANGE_OP.search(comparable):
                yield comparable


def classify(sql, claimed, guess):
    """Which time column one statement predicates on.

    A WHERE slice scores on any mention of the column; an ON clause scores
    only through its range conjuncts, so join keys stay out.
    """
    where_text = " ".join(SINGLE_QUOTED.sub("''", w) for w in where_slices(sql))
    on_text = " ".join(on_range_conjuncts(sql))
    has = lambda column, text: re.search(rf"\b{re.escape(column)}\b", text) is not None
    claimed_hit = has(claimed, where_text) or has(claimed, on_text)
    guess_hit = has(guess, where_text) or has(guess, on_text)
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
        return INCONCLUSIVE, "the executed query's predicates name both time columns"
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
