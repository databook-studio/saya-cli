fn recovery_phase(phase: ProviderRecoveryPhase) -> &'static str {
    match phase {
        ProviderRecoveryPhase::Retrying => "retrying",
        ProviderRecoveryPhase::Exhausted => "exhausted",
        ProviderRecoveryPhase::NotRetried => "not retried",
        _ => "provider recovery",
    }
}

fn recovery_reason(reason: ProviderRecoveryReason) -> &'static str {
    match reason {
        ProviderRecoveryReason::EmptyResponse => "empty response",
        ProviderRecoveryReason::StreamEnded => "stream ended",
        ProviderRecoveryReason::StreamStalled => "stream stalled",
        ProviderRecoveryReason::StreamCut => "stream cut",
        ProviderRecoveryReason::StreamByteLimit => "stream byte limit",
        ProviderRecoveryReason::ToolCallProtocol => "invalid tool-call identifiers",
        ProviderRecoveryReason::OutputTruncated => "output truncated",
        ProviderRecoveryReason::ProviderFailure => "provider failure",
        ProviderRecoveryReason::Cancelled => "cancelled",
        _ => "provider failure",
    }
}
