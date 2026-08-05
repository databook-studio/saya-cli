use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rsa::{
    RsaPrivateKey,
    pkcs1::DecodeRsaPrivateKey,
    pkcs1v15::SigningKey,
    pkcs8::{DecodePrivateKey, EncodePublicKey},
    signature::{SignatureEncoding, Signer},
};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::Keypair;

pub(crate) fn jwt(account: &str, user: &str, key: &Keypair) -> Result<String, ()> {
    let private = parse(&key.private_key, key.passphrase.as_deref())?;
    let public = private
        .to_public_key()
        .to_public_key_der()
        .map_err(|_| ())?;
    let fingerprint =
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(public.as_ref()));
    let subject = format!(
        "{}.{}",
        account.to_ascii_uppercase(),
        user.to_ascii_uppercase()
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ())?
        .as_secs();
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = json!({"iss": format!("{subject}.SHA256:{fingerprint}"), "sub": subject, "iat": now, "exp": now + 3540});
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).map_err(|_| ())?);
    let message = format!("{header}.{body}");
    let signature = SigningKey::<Sha256>::new(private).sign(message.as_bytes());
    Ok(format!(
        "{message}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_vec())
    ))
}

fn parse(value: &str, passphrase: Option<&str>) -> Result<RsaPrivateKey, ()> {
    if let Some(password) = passphrase {
        RsaPrivateKey::from_pkcs8_encrypted_pem(value, password).map_err(|_| ())
    } else {
        RsaPrivateKey::from_pkcs8_pem(value)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(value))
            .map_err(|_| ())
    }
}

#[cfg(test)]
mod tests;
