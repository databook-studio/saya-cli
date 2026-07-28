use serde::{Deserialize, Serialize};

use crate::SqlDialect;

/// A reference to a secret. It is safe to serialize because it holds no value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SecretRef {
    Env { env: String },
    File { file: String },
    Keyring { keyring: String },
}

impl SecretRef {
    pub fn redacted_label(&self) -> String {
        match self {
            Self::Env { env } => format!("env:{env}"),
            Self::File { .. } => "file:[redacted path]".into(),
            Self::Keyring { keyring } => format!("keyring:{keyring}"),
        }
    }
}

/// Typed database connection profile loaded from `connections.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DatabaseProfile {
    #[serde(rename = "postgresql")]
    Postgres {
        host: String,
        port: Option<u16>,
        database: String,
        user: String,
        #[serde(default, rename = "sslmode", alias = "ssl_mode")]
        ssl_mode: Option<PostgresSslMode>,
        password: Option<SecretRef>,
    },
    #[serde(rename = "mysql")]
    Mysql {
        host: String,
        port: Option<u16>,
        database: String,
        user: String,
        #[serde(default, rename = "sslmode", alias = "ssl_mode")]
        ssl_mode: Option<MySqlSslMode>,
        #[serde(default, alias = "ssl_ca")]
        ssl_ca: Option<SecretRef>,
        password: Option<SecretRef>,
    },
    #[serde(rename = "duckdb")]
    DuckDb {
        path: String,
        read_only: Option<bool>,
    },
    #[serde(rename = "snowflake")]
    Snowflake {
        account: String,
        user: String,
        auth_type: SnowflakeAuth,
        private_key: Option<SecretRef>,
        password: Option<SecretRef>,
        passphrase: Option<SecretRef>,
        warehouse: Option<String>,
        database: Option<String>,
        schema: Option<String>,
        role: Option<String>,
    },
}

/// PostgreSQL TLS verification mode. `None` preserves PostgreSQL's `prefer`
/// default for existing profiles that omit `sslmode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostgresSslMode {
    #[serde(rename = "disable")]
    Disable,
    #[serde(rename = "prefer")]
    Prefer,
    #[serde(rename = "require")]
    Require,
    #[serde(rename = "verify-ca", alias = "verify_ca")]
    VerifyCa,
    #[serde(rename = "verify-full", alias = "verify_full")]
    VerifyFull,
}

/// MySQL TLS policy. Omitted legacy profiles are upgraded by the connector to
/// `VerifyIdentity`; this type deliberately has no downgrade-capable default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MySqlSslMode {
    #[serde(rename = "disable")]
    Disable,
    #[serde(rename = "prefer", alias = "preferred")]
    Prefer,
    #[serde(rename = "require")]
    Require,
    #[serde(rename = "verify-ca", alias = "verify_ca")]
    VerifyCa,
    #[serde(rename = "verify-identity", alias = "verify_identity")]
    VerifyIdentity,
}

impl DatabaseProfile {
    pub const fn dialect(&self) -> SqlDialect {
        match self {
            Self::Postgres { .. } => SqlDialect::Postgres,
            Self::Mysql { .. } => SqlDialect::Mysql,
            Self::DuckDb { .. } => SqlDialect::DuckDb,
            Self::Snowflake { .. } => SqlDialect::Snowflake,
        }
    }
}

/// Supported Snowflake authentication flows; browser SSO remains interactive-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnowflakeAuth {
    Keypair,
    Userpass,
    Externalbrowser,
}
