use std::time::Duration;

#[derive(Clone)]
pub struct ProviderSettings {
    pub model: String,
    pub base_url: Option<String>,
    pub timeout: Duration,
}

impl ProviderSettings {
    pub fn new(model: impl Into<String>, base_url: Option<String>) -> Self {
        Self {
            model: model.into(),
            base_url,
            timeout: Duration::from_secs(60),
        }
    }
}

pub(super) fn endpoint(base: Option<&str>, default: &str, suffix: &str) -> String {
    let root = base.unwrap_or(default).trim_end_matches('/');
    format!("{root}/{suffix}")
}
