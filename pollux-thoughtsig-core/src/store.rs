use moka::sync::Cache;
use std::{sync::Arc, time::Duration};

pub type SignatureCacheKey = u64;
pub type SignatureCacheStore = Cache<SignatureCacheKey, Arc<str>>;

#[derive(Clone)]
pub struct MokaSignatureStore {
    cache: SignatureCacheStore,
}

impl MokaSignatureStore {
    pub fn new(ttl_secs: u64, max_capacity: u64) -> Self {
        let cache = Cache::builder()
            .time_to_live(Duration::from_secs(ttl_secs.max(1)))
            .max_capacity(max_capacity.max(1))
            .build();
        Self { cache }
    }

    pub fn get(&self, key: &SignatureCacheKey) -> Option<Arc<str>> {
        self.cache.get(key)
    }

    pub fn put(&self, key: SignatureCacheKey, signature: String) {
        self.cache.insert(key, Arc::from(signature));
    }

    pub fn cache(&self) -> SignatureCacheStore {
        self.cache.clone()
    }
}
