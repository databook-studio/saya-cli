use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rsa::{
    RsaPrivateKey,
    pkcs1v15::{Signature, VerifyingKey},
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding, PrivateKeyInfo, pkcs5},
    signature::Verifier,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Keypair, jwt};

fn claims(token: &str) -> Value {
    serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(token.split('.').nth(1).unwrap())
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn jwt_has_exact_spki_fingerprint_and_valid_rs256_signature() {
    let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    let private_key = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
    let token = jwt(
        "xy12345",
        "jane",
        &Keypair {
            private_key,
            passphrase: None,
        },
    )
    .unwrap();
    let values = claims(&token);
    let spki = key.to_public_key().to_public_key_der().unwrap();
    let fingerprint =
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(spki.as_ref()));
    assert_eq!(values["sub"], "XY12345.JANE");
    assert_eq!(values["iss"], format!("XY12345.JANE.SHA256:{fingerprint}"));
    assert!(values["iat"].as_u64().unwrap() <= values["exp"].as_u64().unwrap());
    assert_eq!(
        values["exp"].as_u64().unwrap() - values["iat"].as_u64().unwrap(),
        3540
    );
    let mut parts = token.split('.');
    let message = format!("{}.{}", parts.next().unwrap(), parts.next().unwrap());
    let signature = Signature::try_from(
        URL_SAFE_NO_PAD
            .decode(parts.next().unwrap())
            .unwrap()
            .as_slice(),
    )
    .unwrap();
    VerifyingKey::<Sha256>::new(key.to_public_key())
        .verify(message.as_bytes(), &signature)
        .unwrap();
}

#[test]
fn encrypted_pkcs8_works_and_wrong_passphrase_is_redacted() {
    let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    let params = pkcs5::pbes2::Parameters::scrypt_aes256cbc(
        pkcs5::scrypt::Params::new(10, 1, 1, 32).unwrap(),
        &[7u8; 8],
        &[9u8; 16],
    )
    .unwrap();
    let private_key = PrivateKeyInfo::try_from(key.to_pkcs8_der().unwrap().as_bytes())
        .unwrap()
        .encrypt_with_params(params, "correct")
        .unwrap()
        .to_pem("ENCRYPTED PRIVATE KEY", LineEnding::LF)
        .unwrap()
        .to_string();
    assert!(
        jwt(
            "acct",
            "user",
            &Keypair {
                private_key: private_key.clone(),
                passphrase: Some("correct".into())
            }
        )
        .is_ok()
    );
    assert!(
        jwt(
            "acct",
            "user",
            &Keypair {
                private_key,
                passphrase: Some("wrong".into())
            }
        )
        .is_err()
    );
}
