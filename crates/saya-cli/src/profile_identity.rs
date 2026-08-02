use sha2::{Digest, Sha256};

pub(crate) fn profile_identity(name: &str) -> String {
    let digest = Sha256::digest(name.as_bytes());
    let mut value = String::from("p-");
    for byte in digest {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

#[cfg(test)]
mod tests {
    #[test]
    fn identity_is_stable_and_opaque() {
        let value = super::profile_identity("quoted profile / password");
        assert_eq!(value.len(), 66);
        assert!(!value.contains("password"));
    }
}
