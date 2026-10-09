//! The SQL family's shared token: one `sql:<connection>` grant covers every
//! read-shaped tool on that connection.

/// The read-shaped SQL family: the tools whose call is one parsed
/// read-only statement against the named connection — keyed on the effect
/// fields (`external_side_effect: false`, `requires_approval: true`,
/// `local_state: None`) plus the `sql` + optional `connection` arguments,
/// not on which block of the toolset builder they sit in. One token,
/// `sql:<connection>`, covers every member on that connection. The
/// suggester's tests derive this list from the definitions themselves, so
/// a tool whose declared shape changes breaks the test rather than silently
/// joining or leaving the family.
///
/// Not in the family, by design: `bounded_sql_query_all` (its referent can
/// grow after approval — `/connect` mid-session would put a database inside
/// a grant made before it existed), `render_chart` (`external_side_effect:
/// true` — it writes a file and opens a browser, which the token's words do
/// not say), and everything with `requires_approval: false` (never asked,
/// so nothing to grant).
pub(crate) const SQL_FAMILY: &[&str] = &[
    "bounded_sql_query",
    "result_shape",
    "column_health",
    "join_check",
];
