use crate::store::SignatureCacheKey;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FillAction {
    Keep,
    UseCached(Arc<str>),
    UseDummy,
}

#[derive(Debug, Clone)]
pub struct FillDecision {
    pub action: FillAction,
    pub key: Option<SignatureCacheKey>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FillStats {
    pub total_considered: usize,
    pub kept_existing: usize,
    pub cache_hits: usize,
    pub dummy_filled: usize,
}
