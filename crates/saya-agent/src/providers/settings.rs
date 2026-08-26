use std::time::Duration;

#[derive(Clone)]
pub struct ProviderSettings {
    pub model: String,
    pub base_url: Option<String>,
    /// Deadline for establishing a request or completing a non-streaming
    /// response. Streaming responses are bounded per chunk instead, so a long
    /// healthy stream is never killed by a total cap.
    pub timeout: Duration,
    /// Maximum gap between stream chunks before the provider is considered
    /// stalled. This — not a total-duration cap — is what bounds streams.
    pub idle_timeout: Duration,
    pub retry_delays: Vec<Duration>,
    /// Sampling temperature sent to every provider that supports it.
    pub temperature: f32,
    /// Per-response output-token ceiling requested from the provider.
    pub max_output_tokens: u32,
}

impl ProviderSettings {
    pub fn new(model: impl Into<String>, base_url: Option<String>) -> Self {
        Self {
            model: model.into(),
            base_url,
            timeout: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(90),
            retry_delays: vec![
                Duration::from_millis(250),
                Duration::from_millis(500),
                Duration::from_millis(1000),
            ],
            temperature: 0.1,
            max_output_tokens: 4096,
        }
    }

    pub fn with_retry_delays(mut self, retry_delays: Vec<Duration>) -> Self {
        self.retry_delays = retry_delays;
        self
    }

    pub fn with_idle_timeout(mut self, idle_timeout: Duration) -> Self {
        self.idle_timeout = idle_timeout;
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.max_output_tokens = max_output_tokens;
        self
    }
}

pub(super) fn endpoint(base: Option<&str>, default: &str, suffix: &str) -> String {
    let root = base.unwrap_or(default).trim_end_matches('/');
    format!("{root}/{suffix}")
}
