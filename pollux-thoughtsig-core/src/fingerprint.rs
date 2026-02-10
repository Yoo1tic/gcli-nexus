use crate::store::SignatureCacheKey;
use ahash::AHasher;
use serde::Serialize;
use std::hash::Hasher;

#[derive(Debug, Default, Clone, Copy)]
pub struct CacheKeyGenerator;

impl CacheKeyGenerator {
    pub fn generate_text(text: impl AsRef<str>) -> Option<SignatureCacheKey> {
        let trimmed = text.as_ref().trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut hasher = AHasher::default();
        hasher.write(trimmed.as_bytes());
        Some(hasher.finish())
    }

    pub fn generate_json(value: &impl Serialize) -> Option<SignatureCacheKey> {
        let mut normalized = serde_json::to_value(value).ok()?;
        normalized.sort_all_objects();

        let mut hasher = AHasher::default();
        hasher.write(normalized.to_string().as_bytes());
        Some(hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn object_key_order_produces_same_fingerprint() {
        let lhs = json!({
            "name": "get_weather",
            "args": { "city": "Berlin", "unit": "c" }
        });
        let rhs = json!({
            "args": { "unit": "c", "city": "Berlin" },
            "name": "get_weather"
        });

        assert_eq!(
            CacheKeyGenerator::generate_json(&lhs),
            CacheKeyGenerator::generate_json(&rhs)
        );
    }

    #[test]
    fn array_order_changes_fingerprint() {
        let lhs = json!(["a", "b"]);
        let rhs = json!(["b", "a"]);

        assert_ne!(
            CacheKeyGenerator::generate_json(&lhs),
            CacheKeyGenerator::generate_json(&rhs)
        );
    }

    #[test]
    fn string_input_is_trimmed_before_hashing() {
        let lhs = "  alpha  ";
        let rhs = "alpha";

        assert_eq!(
            CacheKeyGenerator::generate_text(lhs),
            CacheKeyGenerator::generate_text(rhs)
        );
    }

    #[test]
    fn empty_string_returns_none() {
        assert_eq!(CacheKeyGenerator::generate_text("   "), None);
    }
}
