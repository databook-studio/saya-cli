//! Actual provider wire configuration, not a tokenizer estimate or grant.
#[derive(Clone)]
pub struct PhysicalRequestMetadata {
    provider: &'static str,
    model: String,
    endpoint: String,
    output_tokens: u32,
}
impl PhysicalRequestMetadata {
    pub fn new(
        provider: &'static str,
        model: &str,
        endpoint: &str,
        output_tokens: u32,
    ) -> Result<Self, crate::EffectOwnerFailure> {
        if provider.is_empty()
            || provider.len() > 64
            || model.is_empty()
            || model.len() > 2048
            || endpoint.is_empty()
            || endpoint.len() > 8192
            || output_tokens == 0
        {
            return Err(crate::EffectOwnerFailure::new(std::io::Error::other(
                "invalid physical request binding",
            )));
        }
        Ok(Self {
            provider,
            model: model.to_owned(),
            endpoint: endpoint.to_owned(),
            output_tokens,
        })
    }
    pub fn provider(&self) -> &str {
        self.provider
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn output_tokens(&self) -> u32 {
        self.output_tokens
    }
}
impl std::fmt::Debug for PhysicalRequestMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PhysicalRequestMetadata { endpoint: [redacted] }")
    }
}
