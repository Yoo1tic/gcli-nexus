use pollux_schema::gemini::{GeminiGenerateContentRequest, Part};
use pollux_thoughtsig_core::{
    PatchEvent, PatchOutcome, ThoughtSigPatchable, ThoughtSignatureEngine,
};
use tracing::debug;

// Minimal wrapper for `Part` due to orphan rule:
// we cannot implement `ThoughtSigPatchable` directly on schema types
// from another crate.
struct GeminiPartPatch<'a>(&'a mut Part);

impl GeminiPartPatch<'_> {
    fn signature_preview(&self) -> String {
        self.0
            .thought_signature
            .as_deref()
            .map(preview_signature)
            .unwrap_or_default()
    }
}

impl ThoughtSigPatchable for GeminiPartPatch<'_> {
    fn data(&self) -> PatchEvent<'_> {
        // Priority: functionCall first, then thought text.
        // A thought part without text is still patchable and falls back
        // to dummy signature through empty-text key generation.
        if let Some(function_call) = self.0.function_call.as_ref() {
            return PatchEvent::FunctionCall(function_call);
        }

        if self.0.thought == Some(true) {
            if let Some(text) = self.0.text.as_deref() {
                return PatchEvent::ThoughtText(text);
            }
            return PatchEvent::ThoughtText("");
        }

        PatchEvent::None
    }

    fn thought_signature_mut(&mut self) -> &mut Option<String> {
        self.0.thought_signature_mut()
    }
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

        for (part_idx, part) in content.parts.iter_mut().enumerate() {
            let mut part_patch = GeminiPartPatch(part);
            let applied = part_patch.patch_thought_signature(engine);

            let key = match applied {
                PatchOutcome::Skipped => continue,
                PatchOutcome::Patched { cache_key } => cache_key,
            };

            debug!(
                channel = "geminicli",
                thoughtsig.phase = "fill",
                content_idx = content_idx,
                part_idx = part_idx,
                key = ?key,
                signature = %part_patch.signature_preview(),
                "Thought signature decision"
            );
        }
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
        assert_eq!(
            request.contents[1].parts[0].thought_signature.as_deref(),
            Some("skip_thought_signature_validator")
        );
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
}
