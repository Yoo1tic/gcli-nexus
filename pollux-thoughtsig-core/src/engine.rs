use moka::sync::Cache;
use std::{sync::Arc, time::Duration};

pub type CacheKey = u64;
pub type ThoughtSignature = Arc<str>;
pub type SignatureCacheStore = Cache<CacheKey, ThoughtSignature>;

pub struct ThoughtSignatureEngine {
    cache: SignatureCacheStore,
    dummy_signature: ThoughtSignature,
}

impl ThoughtSignatureEngine {
    pub fn new(ttl_secs: u64, max_capacity: u64) -> Self {
        let cache = SignatureCacheStore::builder()
            .time_to_live(Duration::from_secs(ttl_secs.max(1)))
            .max_capacity(max_capacity.max(1))
            .build();
        let dummy_signature: ThoughtSignature = Arc::from("skip_thought_signature_validator");

        Self {
            cache,
            dummy_signature,
        }
    }

    pub fn get_signature(&self, key: &CacheKey) -> Option<ThoughtSignature> {
        self.cache.get(key)
    }

    pub fn put_signature(&self, key: CacheKey, signature: ThoughtSignature) {
        self.cache.insert(key, signature);
    }

    pub fn fallback_signature(&self) -> ThoughtSignature {
        self.dummy_signature.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_signature_returns_none_when_no_cache() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let key = 42_u64;

        let signature = engine.get_signature(&key);
        assert!(signature.is_none());
    }

    #[test]
    fn get_signature_hits_cache_when_present() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let key = 7_u64;
        engine.put_signature(key, Arc::from("sig_007"));

        let signature = engine.get_signature(&key);
        assert_eq!(signature.as_deref(), Some("sig_007"));
    }
}
