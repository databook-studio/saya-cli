mod keypair;

pub(crate) use keypair::jwt;

#[derive(Clone)]
pub(crate) enum Auth {
    Keypair(Keypair),
    Userpass(Userpass),
    ExternalBrowser(ExternalBrowser),
}

#[derive(Clone)]
pub(crate) struct ExternalBrowser {
    pub(crate) enabled: bool,
    pub(crate) token: std::sync::Arc<tokio::sync::Mutex<Option<String>>>,
}

#[derive(Clone)]
pub(crate) struct Keypair {
    pub(crate) private_key: String,
    pub(crate) passphrase: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Userpass {
    pub(crate) password: String,
    pub(crate) token: std::sync::Arc<tokio::sync::Mutex<Option<String>>>,
}
