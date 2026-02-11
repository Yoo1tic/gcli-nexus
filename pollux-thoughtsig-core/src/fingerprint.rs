use crate::CacheKey;

use ahash::AHasher;
use serde::Serialize;
use std::hash::Hasher;

const DOMAIN_TEXT: u8 = 1;
const DOMAIN_JSON: u8 = 2;

#[derive(Debug, Default, Clone, Copy)]
pub struct CacheKeyGenerator;

impl CacheKeyGenerator {
    pub fn generate_text(text: impl AsRef<str>) -> Option<CacheKey> {
        Some(text.as_ref())
            .filter(|&t| !t.trim().is_empty())
            .map(|t| {
                let mut hasher = AHasher::default();
                hasher.write_u8(DOMAIN_TEXT);
                hasher.write(t.as_bytes());
                hasher.finish()
            })
    }

    pub fn generate_json(value: &impl Serialize) -> Option<CacheKey> {
        let mut normalized = serde_json::to_value(value).ok()?;
        if normalized.is_null() {
            return None;
        }
        normalized.sort_all_objects();
        let bytes = serde_json::to_vec(&normalized).ok()?;

        let mut hasher = AHasher::default();
        hasher.write_u8(DOMAIN_JSON);
        hasher.write(&bytes);
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
