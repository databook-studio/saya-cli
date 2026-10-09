// small schema helpers
// ---------------------------------------------------------------------------

fn schema_tree_with(
    identity: &ProfileIdentity,
    table: &str,
    cols: &[(&str, &str, bool)],
) -> (ProfileIdentity, SchemaTree) {
    (identity.clone(), schema_tree_named(table, cols))
}

fn schema_tree_named(table: &str, cols: &[(&str, &str, bool)]) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![table_named_with(table, cols)],
            }],
        }],
    }
}

fn orders_schema_named(identity: &ProfileIdentity, name: &str) -> (ProfileIdentity, SchemaTree) {
    (
        identity.clone(),
        schema_tree_named(
            name,
            &[("id", "bigint", false), ("created_at", "timestamp", false)],
        ),
    )
}

fn table_named(name: &str) -> Table {
    table_named_with(
        name,
        &[("id", "bigint", false), ("created_at", "timestamp", false)],
    )
}

fn table_named_with(name: &str, cols: &[(&str, &str, bool)]) -> Table {
    Table {
        name: name.into(),
        columns: cols
            .iter()
            .map(|(n, t, nullable)| Column {
                name: (*n).into(),
                data_type: (*t).into(),
                nullable: *nullable,
            })
            .collect(),
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

fn many_orders_schema(identity: &ProfileIdentity, count: usize) -> (ProfileIdentity, SchemaTree) {
    let tables: Vec<Table> = (0..count)
        .map(|i| {
            table_named_with(
                &format!("orders{i}"),
                &[("id", "bigint", false), ("created_at", "timestamp", false)],
            )
        })
        .collect();
    (
        identity.clone(),
        SchemaTree {
            databases: vec![Database {
                name: "catalog".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables,
                }],
            }],
        },
    )
}

// ---------------------------------------------------------------------------
// Phase 5e: conflicts surface in the prompt context block.
//
// recall already detects conflicts (2b-1) and returns them on the contract;
// the render layer ignored that field. These tests pin the surfacing: each
// disputed claim is marked in-band, the kind is named once per contract, and
// the block tells the model not to choose between them silently. A contract
// with no conflict still carries no dispute marker and no do-not-choose
// instruction — the byte-identity to a pre-memory prompt that an earlier slice
// asserted here was given up in P2a (a confirmed claim now binds, so it gains
// a stanza directive and a `[confirmed]` marker); see
// `confirmed_contract_renders_with_directive_and_marker` for the shape it has
// now, and the per-test accounting in the report for what survived.
// ---------------------------------------------------------------------------
