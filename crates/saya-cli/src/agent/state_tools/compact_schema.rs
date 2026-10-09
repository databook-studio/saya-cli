/// Rows returned to the MODEL from a tool call are capped small: the model
/// reasons over a sample and should use aggregate SQL for counts, so feeding it
/// hundreds of rows only bloats context and slows every later turn.
const MODEL_ROW_CAP: usize = 50;

/// The model-facing row cap for one SQL call: the configured cap, narrowed to
/// [`MODEL_ROW_CAP`]. The query path applies exactly this number
/// (`query`), so the approval prompt states the same figure by calling the
/// same function — the prompt's bound and the enforcement cannot drift.
pub(crate) fn model_row_cap(max_rows: usize) -> usize {
    max_rows.min(MODEL_ROW_CAP)
}

/// One table's worth of rendered schema, gathered before keys are resolved so
/// a foreign key can find its target anywhere in the tree.
struct CompactEntry {
    database: String,
    schema: String,
    name: String,
    short: String,
    full: String,
    columns: String,
    primary_key: Vec<String>,
    foreign_keys: Vec<ForeignKey>,
}

/// Flattens a schema into a compact `{ "tables": { name: "..." } }` map —
/// far smaller than the full serialized tree, which otherwise rides in
/// context on every subsequent agent turn.
/// The schema as the model sees it: one entry per table, keyed by the name it
/// should write in SQL.
///
/// The key is an example the model copies, so its depth follows the engine
/// rather than the tree. Every engine is discovered as catalog → schema →
/// table, but SQLite parses only the table name and MySQL only
/// `database.table`; keying those three-deep hands the model a name its own
/// engine rejects, costing a failed statement before it retries. Where the
/// engine's depth cannot separate two tables, both keep the full name — a name
/// that needs correcting beats a table silently missing from the schema.
///
/// Each value is the columns (`name:type`, comma-separated) followed by the
/// primary key and foreign keys only when present, so a table with neither
/// serializes exactly as before. Foreign keys name their target by the same
/// key it appears under elsewhere in this map, so the model can resolve a
/// join target without guessing which same-named table is meant.
fn compact_schema(schema: &SchemaTree, dialect: SqlDialect) -> serde_json::Value {
    let mut entries: Vec<CompactEntry> = Vec::new();
    for database in &schema.databases {
        for schema_ns in &database.schemas {
            for table in &schema_ns.tables {
                let full = format!("{}.{}.{}", database.name, schema_ns.name, table.name);
                let short = match dialect.sql_name_parts() {
                    1 => table.name.clone(),
                    2 => format!("{}.{}", database.name, table.name),
                    _ => full.clone(),
                };
                let columns = table
                    .columns
                    .iter()
                    .map(|column| format!("{}:{}", column.name, column.data_type))
                    .collect::<Vec<_>>()
                    .join(", ");
                entries.push(CompactEntry {
                    database: database.name.clone(),
                    schema: schema_ns.name.clone(),
                    name: table.name.clone(),
                    short,
                    full,
                    columns,
                    primary_key: table.primary_key.clone(),
                    foreign_keys: table.foreign_keys.clone(),
                });
            }
        }
    }

    let key_of = |entry: &CompactEntry| -> String {
        let ambiguous = entries
            .iter()
            .filter(|other| other.short == entry.short)
            .count()
            > 1;
        if ambiguous {
            entry.full.clone()
        } else {
            entry.short.clone()
        }
    };
    let keys: Vec<String> = entries.iter().map(key_of).collect();

    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        by_name.entry(entry.name.as_str()).or_default().push(index);
    }

    let mut tables = serde_json::Map::new();
    for (index, entry) in entries.iter().enumerate() {
        let mut value = entry.columns.clone();
        if !entry.primary_key.is_empty() {
            value.push_str(" | pk=");
            value.push_str(&entry.primary_key.join(","));
        }
        if !entry.foreign_keys.is_empty() {
            value.push_str(" | fk=");
            let rendered: Vec<String> = entry
                .foreign_keys
                .iter()
                .map(|fk| {
                    let referenced_key =
                        resolve_referenced_key(fk, index, &entries, &keys, &by_name);
                    format!(
                        "{}->{}.{}",
                        column_list(&fk.columns),
                        referenced_key,
                        column_list(&fk.referenced_columns)
                    )
                })
                .collect();
            value.push_str(&rendered.join(";"));
        }
        tables.insert(keys[index].clone(), serde_json::Value::String(value));
    }

    serde_json::json!({ "tables": tables })
}

/// One column renders bare; several render as a parenthesised, comma-separated
/// list so a composite key's column order stays visible and pairable.
fn column_list(columns: &[String]) -> String {
    if columns.len() == 1 {
        columns[0].clone()
    } else {
        format!("({})", columns.join(","))
    }
}

/// The key the referenced table appears under in the rendered map. Prefers the
/// schema the connector named, then the referencing table's own schema, then
/// any table of that name; falls back to the bare name the connector reported
/// when the target is not in the tree at all.
fn resolve_referenced_key(
    fk: &ForeignKey,
    referencing_index: usize,
    entries: &[CompactEntry],
    keys: &[String],
    by_name: &HashMap<&str, Vec<usize>>,
) -> String {
    let candidates = by_name
        .get(fk.referenced_table.as_str())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let referencing = &entries[referencing_index];
    let chosen = match &fk.referenced_schema {
        Some(schema) => candidates
            .iter()
            .copied()
            .find(|&i| entries[i].schema == *schema),
        None => candidates
            .iter()
            .copied()
            .find(|&i| {
                entries[i].database == referencing.database
                    && entries[i].schema == referencing.schema
            })
            .or_else(|| {
                candidates
                    .iter()
                    .copied()
                    .find(|&i| entries[i].database == referencing.database)
            }),
    };
    chosen
        .map(|i| keys[i].clone())
        .unwrap_or_else(|| fk.referenced_table.clone())
}
