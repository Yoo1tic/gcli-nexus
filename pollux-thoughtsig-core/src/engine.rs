use crate::{
    fingerprint::CacheKeyGenerator,
    policy::EnginePolicy,
    store::{MokaSignatureStore, SignatureCacheKey},
    types::{FillAction, FillDecision, FillStats},
};
use serde_json::Value;

pub struct ThoughtSignatureEngine {
    store: MokaSignatureStore,
    policy: EnginePolicy,
}

impl ThoughtSignatureEngine {
    pub fn new(store: MokaSignatureStore, policy: EnginePolicy) -> Self {
        Self { store, policy }
    }

    pub fn dummy_signature(&self) -> &str {
        self.policy.dummy_signature.as_str()
    }

    pub fn fill_one(
        &self,
        key_input: Option<&Value>,
        existing_signature: Option<&str>,
        required: bool,
    ) -> FillDecision {
        let key = self.make_key(key_input);

        if existing_signature.is_some() && self.policy.trust_existing {
            return FillDecision {
                action: FillAction::Keep,
                key,
            };
        }

        if !required || !self.policy.fill_missing {
            return FillDecision {
                action: FillAction::Keep,
                key,
            };
        }

        if let Some(cache_key) = key.as_ref() {
            if let Some(sig) = self.store.get(cache_key) {
                return FillDecision {
                    action: FillAction::UseCached(sig),
                    key,
                };
            }
        }

        FillDecision {
            action: FillAction::UseDummy,
            key,
        }
    }

    pub fn classify_fill(decisions: &[FillDecision]) -> FillStats {
        let mut stats = FillStats::default();
        for decision in decisions {
            stats.total_considered += 1;
            match decision.action {
                FillAction::Keep => stats.kept_existing += 1,
                FillAction::UseCached(_) => stats.cache_hits += 1,
                FillAction::UseDummy => stats.dummy_filled += 1,
            }
        }
        stats
    }

    pub fn make_key(&self, key_input: Option<&Value>) -> Option<SignatureCacheKey> {
        match key_input {
            Some(Value::String(text)) => CacheKeyGenerator::generate_text(text),
            Some(value) => CacheKeyGenerator::generate_json(value),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_one_uses_dummy_when_no_cache() {
        let store = MokaSignatureStore::new(3600, 1024);
        let engine = ThoughtSignatureEngine::new(store, EnginePolicy::default());

        let decision = engine.fill_one(Some(&Value::String("abc".to_string())), None, true);
        assert!(matches!(decision.action, FillAction::UseDummy));
    }
}
