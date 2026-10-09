fn recall_request<'a>(
    profiles: &'a [ProfileIdentity],
    schemas: &'a [(ProfileIdentity, SchemaAvailability)],
    terms: &'a [String],
    allow_database_context: bool,
    bounds: RecallBounds,
) -> RecallRequest<'a> {
    RecallRequest {
        profiles,
        explicit_refs: &[],
        terms,
        allow_database_context,
        schemas,
        now_unix_ms: FRESH_NOW,
        bounds,
        // The existing recall tests model today's behaviour: confirmed only.
        // A test that needs `IncludeCandidates` builds its own request.
        recall_mode: RecallMode::Confirmed,
        // No per-claim admission on the legacy recall tests; `None` keeps the
        // mode. The `use_candidate_once` tests build their own request with a
        // real admission.
        admit_candidate: None,
        // The helper defaults to the model-facing policy so the existing tests
        // exercise the same exclusion a real prompt recall applies. A test that
        // wants the human-review path (stale kept) builds its own request.
        policy: RetrievalPolicy::ForModel,
    }
}
