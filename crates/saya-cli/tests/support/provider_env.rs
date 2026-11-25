// The provider variables a child launch must not inherit.
//
// `saya_config`'s first-run auto-choice reads exactly the keys in
// `provider_auto::KEYS`; `SAYA_PROVIDER` states a provider directly. A fixture
// that states no provider has to remove all of them, or a runner with one
// exported chooses that provider and the first-run screen the fixture reads
// never paints. The list is stated here rather than reached through the
// configuration crate: `KEYS` is not part of its public surface.

/// Every provider variable a launch must not inherit: the keys the auto-choice
/// reads (`saya_config::provider_auto::KEYS`, in the same order) and the
/// provider override.
pub(crate) const PROVIDER_ENV: [&str; 6] = [
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "OPENROUTER_API_KEY",
    "DEEPSEEK_API_KEY",
    "SAYA_PROVIDER",
];
