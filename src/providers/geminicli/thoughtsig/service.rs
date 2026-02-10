use super::adapter_request::{apply_request_fill_decisions, collect_request_patch_targets};
use super::adapter_response::GeminiResponseAdapter;
use pollux_schema::gemini::{GeminiGenerateContentRequest, GeminiResponseBody};
use pollux_thoughtsig_core::{
    EnginePolicy, FillAction, FillStats, MokaSignatureStore, SignatureSniffer,
    ThoughtSignatureEngine,
};
use std::sync::Arc;
use tracing::debug;

const DEFAULT_TTL_SECS: u64 = 60 * 60;
const DEFAULT_MAX_CAPACITY: u64 = 200_000;

#[derive(Clone)]
pub struct GeminiThoughtSigService {
    store: MokaSignatureStore,
    engine: Arc<ThoughtSignatureEngine>,
}

impl GeminiThoughtSigService {
    pub fn new() -> Self {
        let store = MokaSignatureStore::new(DEFAULT_TTL_SECS, DEFAULT_MAX_CAPACITY);
        let policy = EnginePolicy::default();
        let engine = ThoughtSignatureEngine::new(store.clone(), policy);

        Self {
            store,
            engine: Arc::new(engine),
        }
    }

    pub fn new_stream_sniffer(&self) -> SignatureSniffer {
        SignatureSniffer::new(self.store.cache())
    }

    pub fn patch_request(
        &self,
        model: &str,
        request: &mut GeminiGenerateContentRequest,
    ) -> FillStats {
        let targets = collect_request_patch_targets(request);
        let mut decisions = Vec::with_capacity(targets.len());

        for target in &targets {
            let decision = self.engine.fill_one(
                target.key_input.as_ref(),
                target.existing_signature.as_deref(),
                true,
            );

            let action = match &decision.action {
                FillAction::Keep => {
                    if target.existing_signature.is_some() {
                        "keep_existing"
                    } else {
                        "keep_noop"
                    }
                }
                FillAction::UseCached(_) => "cache_hit",
                FillAction::UseDummy => "dummy_fill",
            };

            let signature_preview = match &decision.action {
                FillAction::UseCached(signature) => preview_signature(signature),
                FillAction::Keep => target
                    .existing_signature
                    .as_deref()
                    .map(preview_signature)
                    .unwrap_or_else(|| "<none>".to_string()),
                FillAction::UseDummy => self.engine.dummy_signature().to_string(),
            };

            debug!(
                channel = "geminicli",
                thoughtsig.phase = "fill",
                req.model = %model,
                content_idx = target.content_idx,
                part_idx = target.part_idx,
                key = ?decision.key,
                action = action,
                signature = %signature_preview,
                "Thought signature decision"
            );

            decisions.push(decision);
        }

        let stats = ThoughtSignatureEngine::classify_fill(&decisions);
        apply_request_fill_decisions(request, &targets, &decisions, self.engine.dummy_signature());
        stats
    }

    pub fn record_response(&self, response: &GeminiResponseBody) {
        let mut sniffer = self.new_stream_sniffer();
        self.inspect_response_into_sniffer(response, &mut sniffer);
    }

    pub fn record_stream_chunk(
        &self,
        stream_sniffer: &mut SignatureSniffer,
        response: &GeminiResponseBody,
    ) {
        self.inspect_response_into_sniffer(response, stream_sniffer);
    }

    fn inspect_response_into_sniffer(
        &self,
        response: &GeminiResponseBody,
        sniffer: &mut SignatureSniffer,
    ) {
        let adapter = GeminiResponseAdapter(response);
        sniffer.inspect(&adapter);
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
    use serde_json::json;

    #[test]
    fn patch_request_fills_dummy_when_cache_miss() {
        let service = GeminiThoughtSigService::new();
        let mut req: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "thought": true,
                            "text": "internal reasoning"
                        }
                    ]
                }
            ]
        }))
        .expect("request json must parse");

        let stats = service.patch_request("gemini-3-pro-preview", &mut req);
        assert_eq!(stats.total_considered, 1);
        assert_eq!(stats.dummy_filled, 1);
        assert_eq!(
            req.contents[0].parts[0].thought_signature.as_deref(),
            Some("skip_thought_signature_validator")
        );
    }

    #[test]
    fn record_then_patch_hits_cache() {
        let service = GeminiThoughtSigService::new();

        let response: GeminiResponseBody = serde_json::from_value(json!({
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "thought": true,
                                "text": "internal reasoning",
                                "thoughtSignature": "real_signature_123"
                            }
                        ]
                    },
                    "finishReason": "STOP"
                }
            ]
        }))
        .expect("response json must parse");

        service.record_response(&response);

        let mut req: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "thought": true,
                            "text": "internal reasoning"
                        }
                    ]
                }
            ]
        }))
        .expect("request json must parse");

        let fill_stats = service.patch_request("gemini-3-pro-preview", &mut req);
        assert_eq!(fill_stats.cache_hits, 1);
        assert_eq!(fill_stats.dummy_filled, 0);
        assert_eq!(
            req.contents[0].parts[0].thought_signature.as_deref(),
            Some("real_signature_123")
        );
    }

    #[test]
    fn record_then_patch_hits_cache_for_function_call_hash() {
        let service = GeminiThoughtSigService::new();

        let response: GeminiResponseBody = serde_json::from_value(json!({
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "functionCall": {
                                    "name": "get_weather",
                                    "args": {
                                        "city": "Berlin",
                                        "unit": "c"
                                    }
                                },
                                "thoughtSignature": "fn_signature_123"
                            }
                        ]
                    },
                    "finishReason": "STOP"
                }
            ]
        }))
        .expect("response json must parse");

        service.record_response(&response);

        let mut req: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "functionCall": {
                                "name": "get_weather",
                                "args": {
                                    "unit": "c",
                                    "city": "Berlin"
                                }
                            }
                        }
                    ]
                }
            ]
        }))
        .expect("request json must parse");

        let fill_stats = service.patch_request("gemini-3-pro-preview", &mut req);
        assert_eq!(fill_stats.cache_hits, 1);
        assert_eq!(fill_stats.dummy_filled, 0);
        assert_eq!(
            req.contents[0].parts[0].thought_signature.as_deref(),
            Some("fn_signature_123")
        );
    }

    #[test]
    fn stream_record_then_patch_hits_cache_without_role_in_chunk() {
        let service = GeminiThoughtSigService::new();
        let chunk_without_signature: GeminiResponseBody = serde_json::from_value(json!({
            "candidates": [
                {
                    "index": 0,
                    "content": {
                        "parts": [
                            {
                                "thought": true,
                                "text": "alpha "
                            }
                        ]
                    }
                }
            ]
        }))
        .expect("chunk without signature must parse");

        let chunk_with_signature: GeminiResponseBody = serde_json::from_value(json!({
            "candidates": [
                {
                    "index": 0,
                    "finishReason": "STOP",
                    "content": {
                        "parts": [
                            {
                                "thought": true,
                                "text": "beta",
                                "thoughtSignature": "stream_sig_001"
                            }
                        ]
                    }
                }
            ]
        }))
        .expect("chunk with signature must parse");

        let mut stream_sniffer = service.new_stream_sniffer();
        service.record_stream_chunk(&mut stream_sniffer, &chunk_without_signature);
        service.record_stream_chunk(&mut stream_sniffer, &chunk_with_signature);

        let mut req: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": [
                {
                    "role": "model",
                    "parts": [
                        {
                            "thought": true,
                            "text": "alpha beta"
                        }
                    ]
                }
            ]
        }))
        .expect("request json must parse");

        let fill_stats = service.patch_request("gemini-3-pro-preview", &mut req);
        assert_eq!(fill_stats.cache_hits, 1);
        assert_eq!(fill_stats.dummy_filled, 0);
        assert_eq!(
            req.contents[0].parts[0].thought_signature.as_deref(),
            Some("stream_sig_001")
        );
    }
}
