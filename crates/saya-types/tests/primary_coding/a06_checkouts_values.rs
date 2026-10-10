use saya_types::{CheckoutValueError, GitObjectId, ManagedCheckoutPath};
use serde_json::json;

const SHA1_ID: &str = "1111111111111111111111111111111111111111";
const SHA256_ID: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn git_object_ids_accept_only_full_lowercase_sha1_or_sha256_names() {
    let sha1 = GitObjectId::parse(SHA1_ID).unwrap();
    let sha256 = GitObjectId::parse(SHA256_ID).unwrap();
    assert_eq!(sha1.as_str(), SHA1_ID);
    assert_eq!(sha256.as_str(), SHA256_ID);
    assert_eq!(
        serde_json::to_string(&sha1).unwrap(),
        format!("\"{SHA1_ID}\"")
    );

    for invalid in [
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "sha1:1111111111111111111111111111111111111111",
        "1234abcd",
        "gggggggggggggggggggggggggggggggggggggggg",
    ] {
        assert_eq!(
            GitObjectId::parse(invalid),
            Err(CheckoutValueError::InvalidGitObjectId)
        );
        assert!(serde_json::from_str::<GitObjectId>(&json!(invalid).to_string()).is_err());
    }
}

#[test]
fn managed_checkout_paths_are_posix_absolute_lexical_locators() {
    let unicode = ManagedCheckoutPath::new("/private/saya/répo/checkout-1").unwrap();
    assert_eq!(unicode.as_str(), "/private/saya/répo/checkout-1");
    assert_eq!(
        serde_json::to_string(&unicode).unwrap(),
        "\"/private/saya/répo/checkout-1\""
    );

    let exactly_max_bytes = format!("/{}a", "a/".repeat(2047));
    assert_eq!(exactly_max_bytes.len(), 4096);
    assert!(ManagedCheckoutPath::new(&exactly_max_bytes).is_ok());
    assert_eq!(
        ManagedCheckoutPath::new(format!("{exactly_max_bytes}b")),
        Err(CheckoutValueError::InvalidManagedCheckoutPath)
    );

    for invalid in [
        "",
        "/",
        "relative/path",
        "/private/checkout/",
        "/private//checkout",
        "/private/./checkout",
        "/private/../checkout",
        "/private\\checkout",
        "/private/checkout\n",
        "/private/checkout\0",
        "C:/saya/checkout",
        "\\\\server\\saya\\checkout",
    ] {
        assert_eq!(
            ManagedCheckoutPath::new(invalid),
            Err(CheckoutValueError::InvalidManagedCheckoutPath)
        );
        assert!(serde_json::from_str::<ManagedCheckoutPath>(&json!(invalid).to_string()).is_err());
    }
}
