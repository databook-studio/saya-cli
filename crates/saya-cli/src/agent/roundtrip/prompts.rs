//! Prompts for round-trip reconstruction, kept here as constants so the wording
//! can be read and revised without hunting through logic. A future `Restater`
//! implementation substitutes the placeholders before sending: `{sql}`,
//! `{row_count}`, `{columns}` for restate; `{question}`, `{restatement}` for
//! judge. `{columns}` is a comma-separated list of the result's column names.

/// The restate prompt. Shown ONLY the SQL and the result shape, the model is
/// asked to restate — as a single plain-language question — what the SQL
/// computes, and nothing else. The question is deliberately absent: a restater
/// that can see the question will echo it and the round-trip signal disappears.
/// `UNKNOWN` is the model's refusal sentinel; an implementation maps it (and
/// any non-question output) to `None`.
pub(crate) const RESTATE_PROMPT: &str = "\
You are a blind reviewer. You are shown ONLY a SQL statement and the shape of the \
result it produced — a row count and a list of column names. You are NOT shown \
the question that was asked, the schema, or any row values.

Restate, as a single plain-language question, what this SQL computes. Read the \
question off the SQL itself — its columns, tables, filters, grouping, and \
aggregations — not from any outside context.

Output that one question and nothing else:
- no commentary, no explanation, no SQL, no quotation marks;
- no hedging (\"possibly\", \"appears to\", \"might be\");
- a concrete question (e.g. \"how many orders did each customer place?\"), not a \
generic one (\"what does this query return?\").

If the statement cannot be restated as a single concrete question, output \
exactly the word: UNKNOWN

SQL:
{sql}

Result shape: {row_count} row(s), columns: {columns}
";

/// The judge prompt. Shown the question that was asked and the restatement,
/// the model returns YES or NO plus one sentence. It is directed to judge by
/// substance — what is being counted, filtered, or grouped — not by wording:
/// a rephrasing agrees, a different count/filter/group does not. The one
/// sentence is returned whether or not they agree, so a reader can weigh the
/// judgement rather than take a bare boolean.
pub(crate) const JUDGE_PROMPT: &str = "\
You are judging whether a restatement answers the SAME question that was asked.

Question that was asked:
{question}

Restatement of what the SQL computes:
{restatement}

Reply with YES or NO on the first line, then one short sentence on the next \
line saying what differs — or, if they agree, what they both ask.

Judge by substance, not surface form:
- A difference in what is being counted, filtered, or grouped means the questions \
are DIFFERENT — answer NO. (\"names of students with 3 or more friends\" is not \
\"students appearing on either side of a friendship at least 3 times\".)
- A difference only in wording, phrasing, or specificity means the questions are \
the SAME — answer YES. (\"how many orders did each customer place?\" and \
\"count of orders per customer\" are the same question.)
";
