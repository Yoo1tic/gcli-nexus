use pollux_schema::gemini::GeminiGenerateContentRequest;
use pollux_thoughtsig_core::{CacheKeyGenerator, ThoughtSigPatchable, ThoughtSignatureEngine};
use serde_json::Value;
use tracing::debug;

pub(super) struct GeminiRequestAdapter<'a> {
    request: &'a mut GeminiGenerateContentRequest,
}

impl<'a> GeminiRequestAdapter<'a> {
    fn new(request: &'a mut GeminiGenerateContentRequest) -> Self {
        Self { request }
    }
}

impl ThoughtSigPatchable for GeminiRequestAdapter<'_> {
    fn should_patch(&self) -> bool {
        self.request.contents.iter().any(|content| {
            content.role.as_deref() == Some("model")
                && content
                    .parts
                    .iter()
                    .any(|part| part.function_call.is_some() || part.thought == Some(true))
        })
    }

    fn patch_thought_signatures(&mut self, engine: &ThoughtSignatureEngine) {
        if !self.should_patch() {
            return;
        }

        for (content_idx, content) in self.request.contents.iter_mut().enumerate() {
            if content.role.as_deref() != Some("model") {
                continue;
            }

            for (part_idx, part) in content.parts.iter_mut().enumerate() {
                let text_key_input = if part.function_call.is_some() {
                    None
                } else if part.thought == Some(true) {
                    part.text.clone().map(Value::String)
                } else {
                    continue;
                };

                let key_input = part.function_call.as_ref().or(text_key_input.as_ref());
                let key = match key_input {
                    Some(Value::String(text)) => CacheKeyGenerator::generate_text(text),
                    Some(value) => CacheKeyGenerator::generate_json(value),
                    None => None,
                };

                let signature = match key {
                    Some(cache_key) => engine.get_signature(&cache_key),
                    None => engine.default_signature(),
                };
                part.thought_signature = Some(signature.to_string());
                let signature_preview = preview_signature(signature.as_ref());

                debug!(
                    channel = "geminicli",
                    thoughtsig.phase = "fill",
                    content_idx = content_idx,
                    part_idx = part_idx,
                    key = ?key,
                    signature = %signature_preview,
                    "Thought signature decision"
                );
            }
        }
    }
}

pub(super) fn patch_request(
    request: &mut GeminiGenerateContentRequest,
    engine: &ThoughtSignatureEngine,
) {
    let mut adapter = GeminiRequestAdapter::new(request);
    adapter.patch_thought_signatures(engine)
}

fn preview_signature(signature: &str) -> String {
    const MAX: usize = 48;
    if signature.len() <= MAX {
        return signature.to_string();
    }
    format!("{}...", &signature[..MAX])
}
