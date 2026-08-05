use saya_types::PostgresSslMode;

use crate::ConfigError;

pub(crate) fn parse_ssl_mode(value: &str) -> Result<PostgresSslMode, ConfigError> {
    match value {
        "disable" => Ok(PostgresSslMode::Disable),
        "prefer" => Ok(PostgresSslMode::Prefer),
        "require" => Ok(PostgresSslMode::Require),
        "verify-ca" | "verify_ca" => Ok(PostgresSslMode::VerifyCa),
        "verify-full" | "verify_full" => Ok(PostgresSslMode::VerifyFull),
        _ => Err(ConfigError::InvalidEnvironment {
            name: "SAYA_DB_SSLMODE".into(),
            reason: "expected disable, prefer, require, verify-ca, or verify-full".into(),
        }),
    }
}
