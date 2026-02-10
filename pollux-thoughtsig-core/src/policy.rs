#[derive(Debug, Clone)]
pub struct EnginePolicy {
    pub trust_existing: bool,
    pub fill_missing: bool,
    pub dummy_signature: String,
}

impl Default for EnginePolicy {
    fn default() -> Self {
        Self {
            trust_existing: true,
            fill_missing: true,
            dummy_signature: "skip_thought_signature_validator".to_string(),
        }
    }
}
