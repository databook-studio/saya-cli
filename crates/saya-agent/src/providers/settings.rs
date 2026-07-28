use std::time::Duration;

#[derive(Clone)]
pub struct ProviderSettings {
    pub model: String,
    pub base_url: Option<String>,
    pub timeout: Duration,
    pub retry_delays: Vec<Duration>,
}

impl ProviderSettings {
    pub fn new(model: impl Into<String>, base_url: Option<String>) -> Self {
        Self {
            model: model.into(),
            base_url,
            timeout: Duration::from_secs(60),
            retry_delays: vec![Duration::from_millis(10), Duration::from_millis(20)],
        }
    }

    pub fn with_retry_delays(mut self, retry_delays: Vec<Duration>) -> Self {
        self.retry_delays = retry_delays;
        self
    }
}

pub(super) fn endpoint(base: Option<&str>, default: &str, suffix: &str) -> String {
    let root = base.unwrap_or(default).trim_end_matches('/');
    format!("{root}/{suffix}")
}
