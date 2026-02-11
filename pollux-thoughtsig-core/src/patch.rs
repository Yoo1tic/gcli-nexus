use crate::{CacheKey, CacheKeyGenerator, ThoughtSignatureEngine};
use serde_json::Value;

pub enum PatchEvent<'a> {
    ThoughtText(&'a str),
    FunctionCall(&'a Value),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOutcome {
    Skipped,
    Patched { cache_key: Option<CacheKey> },
}

pub trait ThoughtSigPatchable {
    // Provide patch input as a normalized event so the caller does not need
    // to understand the concrete schema layout.
    fn data(&self) -> PatchEvent<'_>;
    // Provide mutable access to the destination signature slot.
    fn thought_signature_mut(&mut self) -> &mut Option<String>;

    // Shared patch pipeline:
    // 1) build cache key from event
    // 2) lookup signature (or fallback to dummy)
    // 3) write back to schema slot
    fn patch_thought_signature(&mut self, engine: &ThoughtSignatureEngine) -> PatchOutcome {
        let cache_key = match self.data() {
            PatchEvent::ThoughtText(text) => CacheKeyGenerator::generate_text(text),
            PatchEvent::FunctionCall(function_call) => {
                CacheKeyGenerator::generate_json(function_call)
            }
            PatchEvent::None => return PatchOutcome::Skipped,
        };

        let signature = match cache_key {
            Some(key) => engine
                .get_signature(&key)
                .unwrap_or_else(|| engine.fallback_signature()),
            None => engine.fallback_signature(),
        };

        *self.thought_signature_mut() = Some(signature.to_string());
        PatchOutcome::Patched { cache_key }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::Arc;

    enum FakeData {
        Text(&'static str),
        FunctionCall(Value),
        None,
    }

    struct FakePatchable {
        data: FakeData,
        signature: Option<String>,
    }

    impl ThoughtSigPatchable for FakePatchable {
        fn data(&self) -> PatchEvent<'_> {
            match &self.data {
                FakeData::Text(text) => PatchEvent::ThoughtText(text),
                FakeData::FunctionCall(function_call) => PatchEvent::FunctionCall(function_call),
                FakeData::None => PatchEvent::None,
            }
        }

        fn thought_signature_mut(&mut self) -> &mut Option<String> {
            &mut self.signature
        }
    }

    #[test]
    fn patch_text_with_cache_hit_uses_cached_signature() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let key = CacheKeyGenerator::generate_text("alpha").expect("text key must exist");
        engine.put_signature(key, Arc::from("sig_alpha"));

        let mut item = FakePatchable {
            data: FakeData::Text("alpha"),
            signature: None,
        };

        let applied = item.patch_thought_signature(&engine);
        assert_eq!(
            applied,
            PatchOutcome::Patched {
                cache_key: Some(key)
            }
        );
        assert_eq!(item.signature.as_deref(), Some("sig_alpha"));
    }

    #[test]
    fn patch_function_call_without_cache_uses_dummy_signature() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let function_call = json!({
            "name": "get_weather",
            "args": { "city": "Berlin" }
        });

        let mut item = FakePatchable {
            data: FakeData::FunctionCall(function_call.clone()),
            signature: None,
        };

        let applied = item.patch_thought_signature(&engine);
        assert_eq!(
            applied,
            PatchOutcome::Patched {
                cache_key: CacheKeyGenerator::generate_json(&function_call),
            }
        );
        assert_eq!(
            item.signature.as_deref(),
            Some("skip_thought_signature_validator")
        );
    }

    #[test]
    fn patch_none_event_is_skipped() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let mut item = FakePatchable {
            data: FakeData::None,
            signature: Some("keep_me".to_string()),
        };

        let applied = item.patch_thought_signature(&engine);
        assert_eq!(applied, PatchOutcome::Skipped);
        assert_eq!(item.signature.as_deref(), Some("keep_me"));
    }

    #[test]
    fn patch_empty_text_uses_dummy_and_none_key() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let mut item = FakePatchable {
            data: FakeData::Text("   "),
            signature: None,
        };

        let applied = item.patch_thought_signature(&engine);
        assert_eq!(applied, PatchOutcome::Patched { cache_key: None });
        assert_eq!(
            item.signature.as_deref(),
            Some("skip_thought_signature_validator")
        );
    }
}
