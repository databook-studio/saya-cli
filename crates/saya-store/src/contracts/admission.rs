//! Structural admission checks for claim payloads.
//!
//! `redact()` is the session-wide marker scrubber: it finds `password=…` shapes and
//! URL credentials. A claim is a narrower thing — one short business statement a
//! human agreed to keep — so it gets a stricter, contract-specific gate that
//! *refuses* rather than scrubs. Storing a scrubbed version would hide the fact that
//! extraction produced something it never should have.
//!
//! What this can and cannot do is worth stating plainly, because the boundary is
//! real: these checks recognise content by *structure*. A PEM header, a SQL
//! statement, an HTTP credential header and an absolute filesystem path all have
//! shapes. An opaque token — a bare password, or a single cell copied out of a
//! result row — has no shape that distinguishes it from a legitimate business term,
//! and no amount of string inspection will separate `hunter2` from a product code.
//! Those are kept out by never putting rows or secrets into a claim in the first
//! place: the typed payload allow-list, and the learning envelope in a later phase.

use crate::StoreError;

/// Rejects a serialized claim payload that structurally resembles a credential,
/// raw SQL, a request header, or a filesystem path.
pub(crate) fn check(serialized: &str) -> Result<(), StoreError> {
    let lower = serialized.to_ascii_lowercase();
    if contains_pem_block(&lower)
        || contains_credential_header(&lower)
        || contains_absolute_path(&lower)
        || contains_sql_statement(&lower)
    {
        return Err(StoreError::Invalid);
    }
    Ok(())
}

/// Any PEM armour. A claim never legitimately carries key material.
fn contains_pem_block(lower: &str) -> bool {
    lower.contains("-----begin ") && lower.contains("key-----")
}

/// `x-api-key: …`, `authorization: …`. The marker list in `redact` only matches the
/// `=` form, so the colon-separated header shape slipped straight through.
fn contains_credential_header(lower: &str) -> bool {
    const HEADERS: [&str; 5] = [
        "x-api-key:",
        "authorization:",
        "proxy-authorization:",
        "x-auth-token:",
        "cookie:",
    ];
    HEADERS.iter().any(|header| lower.contains(header))
}

/// An absolute path points at a machine, not at a database object, and would leak
/// a username or a project layout into shared state.
fn contains_absolute_path(lower: &str) -> bool {
    const ROOTS: [&str; 6] = ["/users/", "/home/", "/var/", "/etc/", "/private/", "/root/"];
    if ROOTS.iter().any(|root| lower.contains(root)) {
        return true;
    }
    // Windows drive-letter paths: a letter, a colon, then a backslash.
    lower
        .as_bytes()
        .windows(3)
        .any(|window| window[0].is_ascii_lowercase() && window[1] == b':' && window[2] == b'\\')
}

/// Learned state omits raw SQL by design, so a payload shaped like a statement is
/// refused. Detection needs *two* co-occurring keywords with word boundaries: a
/// claim may legitimately say "the select box maps to status", and rejecting every
/// claim containing the word "select" would make the feature unusable.
fn contains_sql_statement(lower: &str) -> bool {
    const PAIRS: [(&str, &str); 6] = [
        ("select", "from"),
        ("insert", "into"),
        ("update", "set"),
        ("delete", "from"),
        ("drop", "table"),
        ("union", "select"),
    ];
    PAIRS
        .iter()
        .any(|(first, second)| has_word(lower, first) && has_word(lower, second))
}

fn has_word(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(index, _)| {
        let before_ok = index == 0
            || !haystack.as_bytes()[index - 1].is_ascii_alphanumeric()
                && haystack.as_bytes()[index - 1] != b'_';
        let after = index + needle.len();
        let after_ok = after >= haystack.len()
            || !haystack.as_bytes()[after].is_ascii_alphanumeric()
                && haystack.as_bytes()[after] != b'_';
        before_ok && after_ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_structural_secret_shapes() {
        for bad in [
            "-----BEGIN PRIVATE KEY-----abc",
            "-----BEGIN RSA PRIVATE KEY-----",
            "x-api-key: SENTINELTOKEN",
            "Authorization: Bearer abc",
            "/Users/someone/secret/project",
            "C:\\Users\\someone\\project",
            "SELECT col FROM orders",
            "select  distinct  x  from  t",
            "DROP TABLE orders",
        ] {
            assert_eq!(
                check(bad),
                Err(StoreError::Invalid),
                "should refuse {bad:?}"
            );
        }
    }

    #[test]
    fn admits_ordinary_business_statements() {
        for good in [
            "orders.created_at is the reporting time column",
            "the select box on the orders screen maps to status",
            "one row per shipped order, from the fulfilment system",
            "tier_code 3 means the account is in collections",
            "grain: one row per customer per month",
            "revenue is net of returns",
        ] {
            assert_eq!(check(good), Ok(()), "should admit {good:?}");
        }
    }

    #[test]
    fn a_lone_sql_keyword_is_not_a_statement() {
        // "from" alone, or "select" alone, must not trip the check — business text
        // uses these words constantly.
        assert_eq!(check("orders are shipped from the depot"), Ok(()));
        assert_eq!(check("the user must select a region first"), Ok(()));
    }
}
