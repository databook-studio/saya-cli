use saya_types::MySqlSslMode;

use crate::ConfigError;

pub(super) fn parse_ssl_mode(value: &str) -> Result<MySqlSslMode, ConfigError> {
    match value {
        "disable" => Ok(MySqlSslMode::Disable),
        "prefer" | "preferred" => Ok(MySqlSslMode::Prefer),
        "require" => Ok(MySqlSslMode::Require),
        "verify-ca" | "verify_ca" => Ok(MySqlSslMode::VerifyCa),
        "verify-identity" | "verify_identity" => Ok(MySqlSslMode::VerifyIdentity),
        _ => Err(ConfigError::InvalidEnvironment {
            name: "SAYA_DB_SSLMODE".into(),
            reason: "expected disable, prefer, require, verify-ca, or verify-identity".into(),
        }),
    }
}
