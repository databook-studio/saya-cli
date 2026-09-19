use std::fmt;

use super::SchemaTree;

/// Upper bounds for a schema snapshot crossing a connector or store boundary.
pub const MAX_SCHEMA_DATABASES: usize = 64;
pub const MAX_SCHEMA_SCHEMAS: usize = 256;
pub const MAX_SCHEMA_TABLES: usize = 100_000;
pub const MAX_SCHEMA_COLUMNS: usize = 200_000;
pub const MAX_SCHEMA_NAME_CHARS: usize = 256;
pub const MAX_SCHEMA_BYTES: usize = 16 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaValidationError;

impl fmt::Display for SchemaValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("schema snapshot exceeds a configured limit")
    }
}

impl std::error::Error for SchemaValidationError {}

impl SchemaTree {
    /// Validates aggregate shape and identifier sizes without serializing the
    /// snapshot, so connectors can fail before retaining an unbounded catalog.
    pub fn validate(&self) -> Result<(), SchemaValidationError> {
        if self.databases.len() > MAX_SCHEMA_DATABASES {
            return Err(SchemaValidationError);
        }
        let mut schemas: usize = 0;
        let mut tables: usize = 0;
        let mut columns: usize = 0;
        for database in &self.databases {
            check_name(&database.name)?;
            schemas = schemas.saturating_add(database.schemas.len());
            for schema in &database.schemas {
                check_name(&schema.name)?;
                tables = tables.saturating_add(schema.tables.len());
                for table in &schema.tables {
                    check_name(&table.name)?;
                    columns = columns.saturating_add(table.columns.len());
                    for column in &table.columns {
                        check_name(&column.name)?;
                        check_text(&column.data_type)?;
                    }
                    for key in &table.primary_key {
                        check_name(key)?;
                    }
                    for key in &table.foreign_keys {
                        check_name(&key.referenced_table)?;
                        if key.columns.len() != key.referenced_columns.len() {
                            return Err(SchemaValidationError);
                        }
                        for name in key.columns.iter().chain(&key.referenced_columns) {
                            check_name(name)?;
                        }
                        if let Some(schema) = &key.referenced_schema {
                            check_name(schema)?;
                        }
                    }
                }
            }
        }
        if schemas > MAX_SCHEMA_SCHEMAS
            || tables > MAX_SCHEMA_TABLES
            || columns > MAX_SCHEMA_COLUMNS
        {
            return Err(SchemaValidationError);
        }
        Ok(())
    }
}

fn check_name(value: &str) -> Result<(), SchemaValidationError> {
    if value.is_empty() || value.chars().count() > MAX_SCHEMA_NAME_CHARS {
        Err(SchemaValidationError)
    } else {
        Ok(())
    }
}

fn check_text(value: &str) -> Result<(), SchemaValidationError> {
    (value.chars().count() <= MAX_SCHEMA_NAME_CHARS)
        .then_some(())
        .ok_or(SchemaValidationError)
}
