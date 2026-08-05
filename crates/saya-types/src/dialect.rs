use serde::{Deserialize, Serialize};

/// SQL dialect used for parsing, rendering, and connector behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlDialect {
    #[serde(rename = "postgresql")]
    Postgres,
    Mysql,
    DuckDb,
    Snowflake,
}

impl SqlDialect {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgresql",
            Self::Mysql => "mysql",
            Self::DuckDb => "duckdb",
            Self::Snowflake => "snowflake",
        }
    }
}
