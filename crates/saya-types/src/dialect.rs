use serde::{Deserialize, Serialize};

/// SQL dialect used for parsing, rendering, and connector behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SqlDialect {
    #[serde(rename = "postgresql")]
    Postgres,
    Mysql,
    DuckDb,
    Snowflake,
    Sqlite,
    #[serde(rename = "clickhouse")]
    ClickHouse,
}

impl SqlDialect {
    /// How SQL written for this engine must qualify an object name.
    ///
    /// The depth is not cosmetic: SQLite and MySQL reject a three-part name
    /// outright, so asking for one costs a rejected statement before the
    /// caller corrects itself. Every engine still has a fullest form, which is
    /// what a durable fact should be recorded against.
    pub const fn qualified_name_form(self) -> &'static str {
        match self {
            Self::Postgres | Self::DuckDb | Self::Snowflake => "catalog.schema.object",
            Self::Mysql | Self::ClickHouse => "database.object",
            Self::Sqlite => "object",
        }
    }

    /// How many parts of `catalog.schema.object` this engine's SQL accepts.
    ///
    /// Kept beside [`Self::qualified_name_form`] because the two must agree:
    /// the form is what the model is told, the depth is what it is shown.
    pub const fn sql_name_parts(self) -> usize {
        match self {
            Self::Postgres | Self::DuckDb | Self::Snowflake => 3,
            Self::Mysql | Self::ClickHouse => 2,
            Self::Sqlite => 1,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgresql",
            Self::Mysql => "mysql",
            Self::DuckDb => "duckdb",
            Self::Snowflake => "snowflake",
            Self::Sqlite => "sqlite",
            Self::ClickHouse => "clickhouse",
        }
    }
}
