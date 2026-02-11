use pollux_schema::gemini::{GeminiGenerateContentRequest, Part};
use pollux_thoughtsig_core::{CacheKey, CacheKeyGenerator, ThoughtSignatureEngine};
use tracing::debug;

enum PatchDecision {
    Skipped,
    Patched { cache_key: Option<CacheKey> },
    Dropped { cache_key: Option<CacheKey> },
}

fn patch_part(part: &mut Part, engine: &ThoughtSignatureEngine) -> PatchDecision {
    // Keep the same priority as GeminiCLI: functionCall first, then thought text.
    if let Some(function_call) = part.function_call.as_ref() {
        let cache_key = CacheKeyGenerator::generate_json(function_call);
        if let Some(signature) = cache_key.and_then(|key| engine.get_signature(&key)) {
            *part.thought_signature_mut() = Some(signature.to_string());
            return PatchDecision::Patched { cache_key };
        }

        *part.thought_signature_mut() = Some(engine.fallback_signature().to_string());
        return PatchDecision::Patched { cache_key };
    }

    if part.thought == Some(true) {
        let cache_key = part
            .text
            .as_deref()
            .and_then(CacheKeyGenerator::generate_text);
        let Some(cache_key) = cache_key else {
            return PatchDecision::Dropped { cache_key: None };
        };

        if let Some(signature) = engine.get_signature(&cache_key) {
            *part.thought_signature_mut() = Some(signature.to_string());
            return PatchDecision::Patched {
                cache_key: Some(cache_key),
            };
        }

        return PatchDecision::Dropped {
            cache_key: Some(cache_key),
        };
    }

    PatchDecision::Skipped
}

pub(super) fn patch_request(
    request: &mut GeminiGenerateContentRequest,
    engine: &ThoughtSignatureEngine,
) {
    // Single-pass patch flow:
    // request.contents(model only) -> content.parts -> patch each part.
    // No pre-scan stage is needed.
    for (content_idx, content) in request.contents.iter_mut().enumerate() {
        if content.role.as_deref() != Some("model") {
            continue;
        }

        let mut part_idx = 0usize;
        content.parts.retain_mut(|part| {
            let current_part_idx = part_idx;
            part_idx += 1;

            match patch_part(part, engine) {
                PatchDecision::Skipped => true,
                PatchDecision::Patched { cache_key } => {
                    debug!(
                        channel = "antigravity",
                        thoughtsig.phase = "fill",
                        content_idx = content_idx,
                        part_idx = current_part_idx,
                        key = ?cache_key,
                        signature = %part
                            .thought_signature
                            .as_deref()
                            .map(preview_signature)
                            .unwrap_or_default(),
                        "Thought signature decision"
                    );
                    true
                }
                PatchDecision::Dropped { cache_key } => {
                    debug!(
                        channel = "antigravity",
                        thoughtsig.phase = "drop",
                        content_idx = content_idx,
                        part_idx = current_part_idx,
                        key = ?cache_key,
                        "Drop uncached thought part"
                    );
                    false
                }
            }
        });
    }
}

fn preview_signature(signature: &str) -> String {
    const MAX: usize = 48;
    if signature.len() <= MAX {
        return signature.to_string();
    }
    format!("{}...", &signature[..MAX])
}

#[cfg(test)]
mod tests {
    use super::*;
    use pollux_thoughtsig_core::CacheKeyGenerator;
    use serde_json::json;
    use std::sync::Arc;

    fn parse_request(value: serde_json::Value) -> GeminiGenerateContentRequest {
        serde_json::from_value(value).expect("request json must parse")
    }

    #[test]
    fn patch_request_updates_only_model_content_parts() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let mut request = parse_request(json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [
                        {
                            "thought": true,
                            "text": "ignored user thought"
                        }
                    ]
                },
                {
                    "role": "model",
                    "parts": [
                        {
                            "thought": true,
                            "text": "model thought"
                        }
                    ]
                }
            ]
        }));

        patch_request(&mut request, &engine);

        assert!(request.contents[0].parts[0].thought_signature.is_none());
        assert!(request.contents[1].parts.is_empty());
    }

    #[test]
    fn patch_request_uses_cached_signature_for_function_call() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let function_call = json!({
            "name": "get_weather",
            "args": {
                "city": "Berlin",
                "unit": "c"
            }
        });
        let key =
            CacheKeyGenerator::generate_json(&function_call).expect("function call key must exist");
        engine.put_signature(key, Arc::from("sig_fn_001"));

        let mut request = parse_request(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "functionCall": {
                                "args": {
                                    "unit": "c",
                                    "city": "Berlin"
                                },
                                "name": "get_weather"
                            }
                        }
                    ]
                }
            ]
        }));

        patch_request(&mut request, &engine);

        assert_eq!(
            request.contents[0].parts[0].thought_signature.as_deref(),
            Some("sig_fn_001")
        );
    }

    #[test]
    fn patch_request_uses_dummy_signature_for_function_call_cache_miss() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let mut request = parse_request(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "functionCall": {
                                "name": "get_weather",
                                "args": {
                                    "city": "Berlin"
                                }
                            }
                        }
                    ]
                }
            ]
        }));

        patch_request(&mut request, &engine);

        assert_eq!(
            request.contents[0].parts[0].thought_signature.as_deref(),
            Some("skip_thought_signature_validator")
        );
    }

    #[test]
    fn patch_request_skips_non_patchable_parts() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let mut request = parse_request(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "text": "plain model text"
                        }
                    ]
                }
            ]
        }));

        patch_request(&mut request, &engine);
        assert!(request.contents[0].parts[0].thought_signature.is_none());
    }

    #[test]
    fn patch_request_drops_uncached_thought_part() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let mut request = parse_request(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "thought": true,
                            "text": "model thought"
                        }
                    ]
                }
            ]
        }));

        patch_request(&mut request, &engine);
        assert!(request.contents[0].parts.is_empty());
    }

    #[test]
    fn patch_request_keeps_cached_thought_part() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let key = CacheKeyGenerator::generate_text("model thought").expect("text key must exist");
        engine.put_signature(key, Arc::from("sig_thought_001"));

        let mut request = parse_request(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "thought": true,
                            "text": "model thought"
                        }
                    ]
                }
            ]
        }));

        patch_request(&mut request, &engine);

        assert_eq!(request.contents[0].parts.len(), 1);
        assert_eq!(
            request.contents[0].parts[0].thought_signature.as_deref(),
            Some("sig_thought_001")
        );
    }

    #[test]
    fn patch_request_drops_blank_thought_part() {
        let engine = ThoughtSignatureEngine::new(3600, 1024);
        let mut request = parse_request(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "thought": true,
                            "text": "   "
                        }
                    ]
                }
            ]
        }));

        patch_request(&mut request, &engine);
        assert!(request.contents[0].parts.is_empty());
    }
}
