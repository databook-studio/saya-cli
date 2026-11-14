fn request() -> AgentRequest {
    AgentRequest {
        prompt: "answer".into(),
        profile_names: Vec::new(),
        model: "test".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}
