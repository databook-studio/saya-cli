//! Network destinations and host validation for coding approval scopes.

use std::{net::IpAddr, str::FromStr};

use serde::{Deserialize, Serialize};

use super::ApprovalScopeError;

const MAX_NETWORK_HOST_BYTES: usize = 253;

/// The finite network schemes currently supported by coding approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ApprovalNetworkScheme {
    Http,
    Https,
}

/// A credential-free host-only network destination. HTTP and HTTPS use their
/// default effective ports (80 and 443); the runtime must reject other effective ports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalNetworkDestination {
    scheme: ApprovalNetworkScheme,
    host: String,
}

impl ApprovalNetworkDestination {
    pub fn new(
        scheme: ApprovalNetworkScheme,
        host: impl Into<String>,
    ) -> Result<Self, ApprovalScopeError> {
        let host = host.into();
        if host.is_empty() || host.len() > MAX_NETWORK_HOST_BYTES || !host.is_ascii() {
            return Err(ApprovalScopeError::InvalidDestination);
        }
        let host = match IpAddr::from_str(&host) {
            Ok(address) => address.to_string(),
            Err(_) => {
                let dns_host = validate_dns_host(&host)?;
                if has_ambiguous_numeric_dns_form(&dns_host) {
                    return Err(ApprovalScopeError::InvalidDestination);
                }
                dns_host
            }
        };
        Ok(Self { scheme, host })
    }

    pub fn scheme(&self) -> ApprovalNetworkScheme {
        self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNetworkDestination {
    scheme: ApprovalNetworkScheme,
    host: String,
}

impl<'de> Deserialize<'de> for ApprovalNetworkDestination {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = WireNetworkDestination::deserialize(deserializer)?;
        Self::new(wire.scheme, wire.host).map_err(serde::de::Error::custom)
    }
}

fn validate_dns_host(host: &str) -> Result<String, ApprovalScopeError> {
    let host = host.to_ascii_lowercase();
    if host.len() > MAX_NETWORK_HOST_BYTES
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(ApprovalScopeError::InvalidDestination);
    }
    Ok(host)
}

fn has_ambiguous_numeric_dns_form(host: &str) -> bool {
    let Some(final_label) = host.rsplit('.').next() else {
        return false;
    };
    final_label.bytes().all(|byte| byte.is_ascii_digit())
        || final_label
            .strip_prefix("0x")
            .is_some_and(|suffix| suffix.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
