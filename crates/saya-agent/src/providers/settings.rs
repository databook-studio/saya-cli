use super::wire::ReplayShape;
use crate::{ProviderError, ReasoningEffort};
use std::time::Duration;

const MAX_RETRY_DELAYS: usize = 8;
pub(crate) const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

/// The default backoff schedule, in order: exponential growth from half a
/// second, held at the provider layer's 30 s per-delay ceiling. One named
/// table is the single definition behind both the HTTP establishment path
/// (`http.rs`) and the loop's turn-level retry (`loop_runner::receive`).
pub(crate) const DEFAULT_RETRY_DELAYS: [Duration; 8] = [
    Duration::from_millis(500),
    Duration::from_millis(1000),
    Duration::from_millis(2000),
    Duration::from_millis(4000),
    Duration::from_millis(8000),
    Duration::from_millis(16000),
    Duration::from_millis(30000),
    Duration::from_millis(30000),
];

/// The product default total deadline for one model request. Three minutes
/// leaves room for a long healthy generation while still bounding a stream
/// that never ends.
pub(crate) const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// The default window without a content-bearing delta before a stream is
/// aborted as stalled. Longer than the idle timeout: a model may think
/// silently for a while, but not for two minutes while a gateway feeds the
/// socket keep-alives.
pub(crate) const DEFAULT_STALL_TIMEOUT: Duration = Duration::from_secs(120);

/// The configured stall window's accepted range. Below the floor a healthy
/// model that thinks before its first token is cut; above the ceiling the
/// bound stops bounding a request.
pub(crate) const MIN_STALL_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MAX_STALL_TIMEOUT: Duration = Duration::from_secs(600);

/// The default backoff schedule slept between retryable provider failures, in
/// order. An empty list means one attempt with no retries.
pub(crate) fn default_retry_delays() -> Vec<Duration> {
    DEFAULT_RETRY_DELAYS.to_vec()
}

#[derive(Clone)]
pub struct ProviderSettings {
    pub model: String,
    pub base_url: Option<String>,
    /// Deadline for establishing a request or completing a non-streaming
    /// response. Streaming responses are bounded per chunk instead, so a long
    /// healthy stream is never killed by a total cap.
    pub timeout: Duration,
    /// Maximum gap between stream chunks before the provider is considered
    /// stalled. This bounds a silent stream; [`Self::request_timeout`] bounds
    /// one that keeps flowing.
    pub idle_timeout: Duration,
    /// Maximum gap between content-bearing deltas (text, reasoning, or
    /// tool-call arguments) before the request is aborted as stalled. Unlike
    /// [`Self::idle_timeout`], transport keep-alives, empty deltas, and
    /// usage-only frames do not reset it. The setter accepts 30..=600 s.
    pub stall_timeout: Duration,
    /// Total deadline for one logical model request, including a stream that
    /// keeps delivering chunks. On expiry the stream is cut and the loop
    /// continues with its bounded stream-cut notice.
    pub request_timeout: Duration,
    /// Backoff schedule slept between retryable provider failures, in order. An
    /// empty list means one attempt with no retries.
    pub retry_delays: Vec<Duration>,
    /// Sampling temperature sent to every provider that supports it.
    pub temperature: f32,
    /// Per-response output-token ceiling requested from the provider, or
    /// `None` when the operator configured none. `None` sends no limit on the
    /// routes whose API allows omitting one, so the model's own default
    /// applies; Anthropic requires `max_tokens` and falls back to its own.
    pub max_output_tokens: Option<u32>,
    /// How hard the operator asked the model to think, or `None` when they
    /// asked for nothing — the default, and the state in which no provider
    /// sends an effort field at all.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Whether an `openrouter.ai` request may carry OpenRouter's two
    /// app-attribution headers. On by default; off omits them, and no other
    /// host ever receives them.
    pub app_attribution: bool,
}

impl ProviderSettings {
    pub fn new(model: impl Into<String>, base_url: Option<String>) -> Self {
        Self {
            model: model.into(),
            base_url,
            timeout: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(90),
            stall_timeout: DEFAULT_STALL_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            retry_delays: default_retry_delays(),
            temperature: 0.1,
            max_output_tokens: None,
            reasoning_effort: None,
            app_attribution: true,
        }
    }

    /// The effort a request actually goes out with: the caller's own intent
    /// when it stated one, otherwise the operator's configured effort,
    /// otherwise [`ReasoningEffort::Default`] — send nothing.
    ///
    /// The caller wins because a caller that states an effort is stating
    /// something specific about *this* call (the extraction call asks for
    /// `Minimal` so a mechanical JSON answer does not pay for a chain of
    /// thought); the configuration is a default for calls that have no
    /// opinion, which is the agent loop. A configuration that silently
    /// overrode the extraction call would re-inflate every turn's second
    /// request, which is the cost this setting exists to let an operator
    /// manage.
    pub fn effective_reasoning_effort(&self, requested: ReasoningEffort) -> ReasoningEffort {
        match requested {
            ReasoningEffort::Default => self.reasoning_effort.unwrap_or_default(),
            explicit => explicit,
        }
    }

    /// How this route accepts the previous assistant turn's reasoning.
    ///
    /// OpenRouter has its own two-field shape. The native OpenAI endpoint
    /// rejects `reasoning_content` (and o-series models reason already), so a
    /// route with no configured base URL, or an o-series model, sends nothing.
    /// Every other OpenAI-compatible endpoint — DeepSeek, a gateway —
    /// takes the DeepSeek spelling. This is the "where supported" check: a
    /// provider that cannot carry the field simply never receives it.
    pub(crate) fn reasoning_replay(&self) -> ReplayShape {
        if self.is_openrouter() {
            return ReplayShape::OpenRouter;
        }
        if self.base_url.is_none() || is_openai_reasoning_model(&self.model) {
            return ReplayShape::None;
        }
        ReplayShape::ReasoningContent
    }

    pub fn with_retry_delays(mut self, retry_delays: Vec<Duration>) -> Self {
        self.retry_delays = retry_delays;
        self
    }

    pub fn with_idle_timeout(mut self, idle_timeout: Duration) -> Self {
        self.idle_timeout = idle_timeout;
        self
    }

    /// Sets the progress-stall window, rejecting a value outside
    /// [`MIN_STALL_TIMEOUT`]..=[`MAX_STALL_TIMEOUT`].
    pub fn with_stall_timeout(mut self, stall_timeout: Duration) -> Result<Self, ProviderError> {
        if !(MIN_STALL_TIMEOUT..=MAX_STALL_TIMEOUT).contains(&stall_timeout) {
            return Err(ProviderError::Configuration(
                "provider stall timeout is outside its 30..=600 s bound".into(),
            ));
        }
        self.stall_timeout = stall_timeout;
        Ok(self)
    }

    pub fn with_request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
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

    pub fn with_max_output_tokens(mut self, max_output_tokens: Option<u32>) -> Self {
        self.max_output_tokens = max_output_tokens;
        self
    }

    pub fn with_reasoning_effort(mut self, reasoning_effort: Option<ReasoningEffort>) -> Self {
        self.reasoning_effort = reasoning_effort;
        self
    }

    pub fn with_app_attribution(mut self, app_attribution: bool) -> Self {
        self.app_attribution = app_attribution;
        self
    }

    /// Whether this request may carry OpenRouter's app-attribution headers:
    /// the operator left attribution on and the endpoint's host is exactly
    /// `openrouter.ai`.
    pub(crate) fn app_attribution_applies(&self) -> bool {
        self.app_attribution && self.is_openrouter()
    }

    /// Whether the configured base URL's host is exactly `openrouter.ai`.
    /// Its routes take their own request shapes, so the providers check it.
    pub(crate) fn is_openrouter(&self) -> bool {
        self.base_url
            .as_deref()
            .and_then(host)
            .is_some_and(|host| host == "openrouter.ai")
    }

    pub(super) fn validate_retries(&self) -> Result<(), ProviderError> {
        if self.retry_delays.len() > MAX_RETRY_DELAYS
            || self
                .retry_delays
                .iter()
                .any(|delay| *delay > MAX_RETRY_DELAY)
        {
            return Err(ProviderError::Configuration(
                "provider retry schedule exceeds its bound".into(),
            ));
        }
        Ok(())
    }
}

pub(super) fn endpoint(base: Option<&str>, default: &str, suffix: &str) -> String {
    let root = base.unwrap_or(default).trim_end_matches('/');
    format!("{root}/{suffix}")
}

fn host(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok()?.host_str().map(str::to_owned)
}

/// OpenAI's o-series reasons natively and does not take a replayed
/// `reasoning_content` field. Matched by the published family prefix.
fn is_openai_reasoning_model(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    ["o1", "o3", "o4"]
        .iter()
        .any(|prefix| model.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_retry_delays_carries_the_configured_schedule() {
        let settings = ProviderSettings::new("m", None)
            .with_retry_delays(vec![Duration::from_millis(100), Duration::from_millis(200)]);
        assert_eq!(
            settings.retry_delays,
            vec![Duration::from_millis(100), Duration::from_millis(200),]
        );
    }

    /// A caller with no opinion gets the operator's effort; with none
    /// configured either, nothing is sent.
    #[test]
    fn an_unstated_effort_falls_back_to_the_configured_one_or_to_nothing() {
        let unconfigured = ProviderSettings::new("m", None);
        assert_eq!(
            unconfigured.effective_reasoning_effort(ReasoningEffort::Default),
            ReasoningEffort::Default
        );

        let configured =
            ProviderSettings::new("m", None).with_reasoning_effort(Some(ReasoningEffort::High));
        assert_eq!(
            configured.effective_reasoning_effort(ReasoningEffort::Default),
            ReasoningEffort::High
        );
    }

    /// A caller that states an effort is stating something about *this* call:
    /// the extraction call's `Minimal` must survive a configured `High`, or the
    /// setting would re-inflate every turn's second request.
    #[test]
    fn a_stated_effort_is_never_overridden_by_the_configured_one() {
        let configured =
            ProviderSettings::new("m", None).with_reasoning_effort(Some(ReasoningEffort::High));
        assert_eq!(
            configured.effective_reasoning_effort(ReasoningEffort::Minimal),
            ReasoningEffort::Minimal
        );
        assert_eq!(
            configured.effective_reasoning_effort(ReasoningEffort::Low),
            ReasoningEffort::Low
        );
    }

    #[test]
    fn reasoning_replay_follows_the_route() {
        let openrouter = ProviderSettings::new(
            "deepseek/deepseek-chat",
            Some("https://openrouter.ai/api/v1".into()),
        );
        assert_eq!(openrouter.reasoning_replay(), ReplayShape::OpenRouter);

        let compatible =
            ProviderSettings::new("deepseek-chat", Some("https://api.deepseek.com/v1".into()));
        assert_eq!(compatible.reasoning_replay(), ReplayShape::ReasoningContent);

        let native = ProviderSettings::new("gpt-5", None);
        assert_eq!(native.reasoning_replay(), ReplayShape::None);

        let o_series = ProviderSettings::new("o3-mini", Some("https://proxy.example/v1".into()));
        assert_eq!(o_series.reasoning_replay(), ReplayShape::None);
    }

    #[test]
    fn a_request_has_a_total_deadline_of_three_minutes_by_default() {
        assert_eq!(
            ProviderSettings::new("m", None).request_timeout,
            Duration::from_secs(180)
        );
    }

    /// The progress-stall window defaults to two minutes and its setter keeps
    /// a configured value inside the documented 30..=600 s bound.
    #[test]
    fn the_stall_window_defaults_to_two_minutes_and_is_bounded() {
        assert_eq!(
            ProviderSettings::new("m", None).stall_timeout,
            Duration::from_secs(120)
        );
        assert!(
            ProviderSettings::new("m", None)
                .with_stall_timeout(Duration::from_secs(29))
                .is_err(),
            "below the floor a stream can be cut before a model can answer"
        );
        assert!(
            ProviderSettings::new("m", None)
                .with_stall_timeout(Duration::from_secs(601))
                .is_err(),
            "above the ceiling the bound no longer bounds a request"
        );
        let at_floor = ProviderSettings::new("m", None)
            .with_stall_timeout(Duration::from_secs(30))
            .expect("the floor is accepted");
        assert_eq!(at_floor.stall_timeout, Duration::from_secs(30));
        let at_ceiling = ProviderSettings::new("m", None)
            .with_stall_timeout(Duration::from_secs(600))
            .expect("the ceiling is accepted");
        assert_eq!(at_ceiling.stall_timeout, Duration::from_secs(600));
    }

    #[test]
    fn with_request_timeout_carries_the_configured_deadline() {
        let settings =
            ProviderSettings::new("m", None).with_request_timeout(Duration::from_millis(250));
        assert_eq!(settings.request_timeout, Duration::from_millis(250));
    }

    #[test]
    fn new_defaults_to_the_eight_entry_exponential_backoff() {
        let settings = ProviderSettings::new("m", None);
        assert_eq!(settings.retry_delays, DEFAULT_RETRY_DELAYS.to_vec());
        assert_eq!(
            settings.retry_delays,
            vec![
                Duration::from_millis(500),
                Duration::from_millis(1000),
                Duration::from_millis(2000),
                Duration::from_millis(4000),
                Duration::from_millis(8000),
                Duration::from_millis(16000),
                Duration::from_millis(30000),
                Duration::from_millis(30000),
            ]
        );
    }
}
