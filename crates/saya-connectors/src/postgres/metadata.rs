use std::collections::{BTreeMap, HashMap};

use futures_util::TryStreamExt;
use saya_types::{Column, ConnectionError, Database, ForeignKey, Schema, SchemaTree, Table};
use sqlx::Row;
use tokio::time::timeout;

use super::{PostgresConnector, errors};

/// Columns of one foreign-key constraint as its rows arrive: the local
/// columns, the referenced schema and table, and the referenced columns,
/// keyed by the schema, table and constraint the rows belong to.
type ConstraintRows =
    BTreeMap<(String, String, String), (Vec<String>, String, String, Vec<String>)>;

pub(crate) async fn schema(connector: &PostgresConnector) -> Result<SchemaTree, ConnectionError> {
    let database = timeout(
        connector.query_timeout,
        sqlx::query_scalar::<_, String>("SELECT current_database()").fetch_one(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::schema_failed("PostgreSQL schema discovery timed out"))?
    .map_err(errors::schema)?;
    let work = async {
        let mut stream = sqlx::query(SCHEMA_SQL).fetch(&connector.pool);
        let mut schemas = BTreeMap::<String, BTreeMap<String, Vec<Column>>>::new();
        while let Some(row) = stream.try_next().await.map_err(errors::schema)? {
            let schema = row.try_get("table_schema").map_err(errors::row)?;
            let table = row.try_get("table_name").map_err(errors::row)?;
            let column = Column {
                name: row.try_get("column_name").map_err(errors::row)?,
                data_type: row.try_get("data_type").map_err(errors::row)?,
                nullable: row
                    .try_get::<String, _>("is_nullable")
                    .map_err(errors::row)?
                    == "YES",
            };
            schemas
                .entry(schema)
                .or_default()
                .entry(table)
                .or_default()
                .push(column);
        }
        Ok(schemas)
    };
    let schemas = timeout(connector.query_timeout, work)
        .await
        .map_err(|_| ConnectionError::schema_failed("PostgreSQL schema discovery timed out"))??;
    let mut foreign_keys = load_foreign_keys(connector).await?;

    let mut result_schemas: Vec<Schema> = Vec::new();
    for (name, tables) in schemas {
        let mut built_tables: Vec<Table> = Vec::new();
        for (table_name, columns) in tables {
            let foreign_keys = foreign_keys
                .remove(&(name.clone(), table_name.clone()))
                .unwrap_or_default();
            built_tables.push(Table {
                name: table_name,
                columns,
                primary_key: vec![],
                foreign_keys,
            });
        }
        result_schemas.push(Schema {
            name,
            tables: built_tables,
        });
    }
    Ok(SchemaTree {
        databases: vec![Database {
            name: database,
            schemas: result_schemas,
        }],
    })
}

const SCHEMA_SQL: &str = "SELECT c.table_schema, c.table_name, c.column_name, c.data_type, c.is_nullable FROM information_schema.columns c JOIN information_schema.tables t ON t.table_schema = c.table_schema AND t.table_name = c.table_name WHERE c.table_schema NOT LIKE 'pg_%' AND c.table_schema <> 'information_schema' AND t.table_type IN ('BASE TABLE', 'VIEW') ORDER BY c.table_schema, c.table_name, c.ordinal_position";

/// Foreign keys live in `pg_constraint`, where `conkey` and `confkey` are
/// parallel arrays: `conkey[i]` references `confkey[i]`. Unnesting both with
/// ordinality and joining on that ordinal pairs a composite key's columns
/// positionally — the property `information_schema` loses for the referenced
/// side, which has no column that lines up with `position_in_unique_constraint`.
/// One round trip returns every constraint on every table.
const FK_SQL: &str = "SELECT n.nspname AS table_schema, c.relname AS table_name, con.conname AS constraint_name, ref.attname AS column_name, fn.nspname AS ref_schema, fc.relname AS ref_table, fk.attname AS ref_column FROM pg_constraint con JOIN pg_class c ON c.oid = con.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace JOIN pg_class fc ON fc.oid = con.confrelid JOIN pg_namespace fn ON fn.oid = fc.relnamespace JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) ON true JOIN LATERAL unnest(con.confkey) WITH ORDINALITY AS kf(attnum, ord) ON kf.ord = k.ord JOIN pg_attribute ref ON ref.attrelid = con.conrelid AND ref.attnum = k.attnum JOIN pg_attribute fk ON fk.attrelid = con.confrelid AND fk.attnum = kf.attnum WHERE con.contype = 'f' AND n.nspname NOT LIKE 'pg_%' AND n.nspname <> 'information_schema' ORDER BY n.nspname, c.relname, con.conname, k.ord";

async fn load_foreign_keys(
    connector: &PostgresConnector,
) -> Result<HashMap<(String, String), Vec<ForeignKey>>, ConnectionError> {
    let rows = timeout(
        connector.query_timeout,
        sqlx::query(FK_SQL).fetch_all(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::schema_failed("PostgreSQL schema discovery timed out"))?
    .map_err(errors::schema)?;

    // One constraint spans several rows (a column pair each); the constraint
    // name groups them and the query orders the rows so the column vectors
    // come out in declaration order.
    let mut by_constraint: ConstraintRows = BTreeMap::new();
    for row in rows {
        let schema: String = row.try_get("table_schema").map_err(errors::row)?;
        let table: String = row.try_get("table_name").map_err(errors::row)?;
        let constraint: String = row.try_get("constraint_name").map_err(errors::row)?;
        let column: String = row.try_get("column_name").map_err(errors::row)?;
        let ref_schema: String = row.try_get("ref_schema").map_err(errors::row)?;
        let ref_table: String = row.try_get("ref_table").map_err(errors::row)?;
        let ref_column: String = row.try_get("ref_column").map_err(errors::row)?;
        let entry = by_constraint
            .entry((schema, table, constraint))
            .or_insert_with(|| (Vec::new(), ref_schema, ref_table, Vec::new()));
        entry.0.push(column);
        entry.3.push(ref_column);
    }

    let mut map: HashMap<(String, String), Vec<ForeignKey>> = HashMap::new();
    for ((schema, table, _), (columns, ref_schema, ref_table, ref_columns)) in by_constraint {
        map.entry((schema, table)).or_default().push(ForeignKey {
            columns,
            referenced_schema: Some(ref_schema),
            referenced_table: ref_table,
            referenced_columns: ref_columns,
        });
    }
    Ok(map)
}
