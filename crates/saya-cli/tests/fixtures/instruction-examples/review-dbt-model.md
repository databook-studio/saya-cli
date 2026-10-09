# Review one dbt model

You are a read-only reviewer. The user names a dbt model (a `.sql` file, with
its YAML). Judge the model against the project's own conventions and — when
a warehouse profile is connected — against the real warehouse. Never modify
files unless asked; never run a write.

## Read the model in context

1. Read the model SQL with `workspace_read`; find its YAML (schema, tests)
   with `glob` and read it. Grep sibling models for this model's name to see
   how it is consumed.
2. Check `ref()` and `source()`: every relation the model reads should come
   through `ref()` or `source()`. A hardcoded table or schema name is a
   finding — it breaks lineage and environment portability.
3. Check the declared materialisation (`table`, `view`, `incremental`)
   against how the model is used. For `incremental`, look for the guards: an
   `is_incremental()` filter, a stable unique key, and what a late-arriving
   row would do. A model marked incremental with no guard rebuilds wrong.
4. Check tests: `unique` and `not_null` on keys, plus the invariants the
   model promises. A model with no tests ships unverified. Check naming
   against the project's own conventions (`dbt_project.yml`, style docs) and
   quote the convention you applied.

## Verify against the warehouse (only when connected)

5. When a profile exists: `schema_discovery` for the compiled relation, then
   `result_shape` (row count, column types) and a small `bounded_sql_query`
   (with `LIMIT`) to compare the model's output with what its sources imply.
   Read only. Do not run `dbt` itself unless the user asked and the
   host-command lane is enabled (see `HOST-COMMANDS-v1.md`).

## Report

6. Report findings citing the file path and object names, each with a
   concrete fix, and state every assumption you could not verify as an
   assumption.