use serde_json::{Map, Value, json};

use super::client::Context;

pub(crate) fn fields(context: &Context) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("ROLE_NAME".into(), json!(context.role));
    fields.insert("WAREHOUSE_NAME".into(), json!(context.warehouse));
    fields.insert("DATABASE_NAME".into(), json!(context.database));
    fields.insert("SCHEMA_NAME".into(), json!(context.schema));
    fields
}

pub(crate) fn params(context: &Context) -> Vec<(&'static str, &str)> {
    [
        ("warehouse", context.warehouse.as_deref()),
        ("databaseName", context.database.as_deref()),
        ("schemaName", context.schema.as_deref()),
        ("roleName", context.role.as_deref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value
            .filter(|item| !item.is_empty())
            .map(|item| (name, item))
    })
    .collect()
}
