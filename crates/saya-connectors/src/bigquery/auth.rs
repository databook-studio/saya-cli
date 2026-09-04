use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Client;
use rsa::{
    RsaPrivateKey,
    pkcs1v15::SigningKey,
    pkcs8::DecodePrivateKey,
    signature::{SignatureEncoding, Signer},
};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::time::Instant;

/// Read-only BigQuery access. The OAuth scope is a second read-only bound: even
/// a leaked token cannot write, on top of the SQL safety layer's allow-list.
pub(crate) const SCOPE: &str = "https://www.googleapis.com/auth/bigquery.readonly";
const LIFETIME: u64 = 3600;

/// A parsed service-account key. Holds the private key material, so it
/// deliberately does not implement `Debug` — the key and the tokens it mints
/// are secrets and must never reach a log, an error, or a debugger view.
pub(crate) struct ServiceAccount {
    pub(crate) client_email: String,
    pub(crate) private_key: RsaPrivateKey,
    pub(crate) token_uri: String,
}

/// Parses a service-account JSON key. The key is PKCS#8 PEM, the shape Google
/// issues; an encrypted key is rejected because the bearer flow needs to sign
/// without a passphrase prompt. Fields are read by path rather than derived
/// so the connector depends only on `serde_json`, not on `serde` directly.
pub(crate) fn parse_service_account(json: &str) -> Result<ServiceAccount, ()> {
    let raw: Value = serde_json::from_str(json).map_err(|_| ())?;
    let client_email = raw.get("client_email").and_then(Value::as_str).ok_or(())?;
    let private_key_pem = raw.get("private_key").and_then(Value::as_str).ok_or(())?;
    let token_uri = raw.get("token_uri").and_then(Value::as_str).ok_or(())?;
    let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem).map_err(|_| ())?;
    Ok(ServiceAccount {
        client_email: client_email.into(),
        private_key,
        token_uri: token_uri.into(),
    })
}

/// Builds and signs the JWT used for the OAuth2 JWT-bearer grant. The claims
/// match Google's specification: issuer and audience are the service account,
/// the scope is read-only, and the lifetime is one hour.
pub(crate) fn jwt(account: &ServiceAccount) -> Result<String, ()> {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ())?
        .as_secs();
    let claims = json!({
        "iss": account.client_email,
        "scope": SCOPE,
        "aud": account.token_uri,
        "iat": now,
        "exp": now + LIFETIME,
    });
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).map_err(|_| ())?);
    let message = format!("{header}.{body}");
    let signature = SigningKey::<Sha256>::new(account.private_key.clone()).sign(message.as_bytes());
    Ok(format!(
        "{message}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_vec())
    ))
}

/// A cached access token with the instant it should be refreshed. The token
/// is a secret; the field is never formatted or logged.
pub(crate) struct CachedToken {
    pub(crate) token: String,
    pub(crate) expires_at: Instant,
}

impl CachedToken {
    fn fresh(token: String) -> Self {
        // Refresh a minute before the server's expiry so a request issued near
        // the boundary does not reach Google with a token that has just lapsed.
        Self {
            token,
            expires_at: Instant::now() + Duration::from_secs(LIFETIME.saturating_sub(60)),
        }
    }

    pub(crate) fn is_current(&self) -> bool {
        Instant::now() < self.expires_at
    }
}

/// Exchanges a signed JWT for an access token using the JWT-bearer grant. On
/// any failure the caller surfaces the generic auth error — the response body
/// is not read into the message, so a server error cannot echo the token or
/// the assertion back.
pub(crate) async fn exchange(
    client: &Client,
    account: &ServiceAccount,
    deadline: Duration,
) -> Result<CachedToken, ()> {
    let assertion = jwt(account)?;
    let params = [
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", assertion.as_str()),
    ];
    let response = tokio::time::timeout(
        deadline,
        client.post(&account.token_uri).form(&params).send(),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let body: Value = response.json().await.map_err(|_| ())?;
    let token = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or(())?
        .to_owned();
    Ok(CachedToken::fresh(token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use serde_json::Value;

    fn account() -> ServiceAccount {
        let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let private_key = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let json = format!(
            r#"{{"client_email":"reader@proj.iam.gserviceaccount.com","private_key":{private_key:?},"token_uri":"https://oauth2.googleapis.com/token"}}"#
        );
        parse_service_account(&json).unwrap()
    }

    fn claims(token: &str) -> Value {
        serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(token.split('.').nth(1).unwrap())
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn jwt_carries_readonly_scope_and_google_claims() {
        let account = account();
        let token = jwt(&account).unwrap();
        let values = claims(&token);
        assert_eq!(values["iss"], "reader@proj.iam.gserviceaccount.com");
        assert_eq!(values["aud"], "https://oauth2.googleapis.com/token");
        assert_eq!(values["scope"], SCOPE);
        assert!(values["iat"].as_u64().unwrap() <= values["exp"].as_u64().unwrap());
        assert_eq!(
            values["exp"].as_u64().unwrap() - values["iat"].as_u64().unwrap(),
            LIFETIME
        );
    }

    #[test]
    fn jwt_signature_verifies_under_the_public_key() {
        use rsa::pkcs1v15::{Signature, VerifyingKey};
        use rsa::signature::Verifier;
        let account = account();
        let token = jwt(&account).unwrap();
        let mut parts = token.split('.');
        let message = format!("{}.{}", parts.next().unwrap(), parts.next().unwrap());
        let signature = Signature::try_from(
            URL_SAFE_NO_PAD
                .decode(parts.next().unwrap())
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        VerifyingKey::<Sha256>::new(account.private_key.to_public_key())
            .verify(message.as_bytes(), &signature)
            .unwrap();
    }

    #[test]
    fn malformed_key_json_is_rejected_without_panic() {
        assert!(parse_service_account("not json").is_err());
        assert!(parse_service_account("{}").is_err());
    }

    #[test]
    fn cached_token_is_current_then_expires() {
        let token = CachedToken::fresh("t".into());
        assert!(token.is_current());
        let stale = CachedToken {
            token: "t".into(),
            expires_at: Instant::now() - Duration::from_secs(1),
        };
        assert!(!stale.is_current());
    }
}
