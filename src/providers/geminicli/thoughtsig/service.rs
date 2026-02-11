use super::adapter_request::patch_request;
use super::adapter_response::GeminiResponseAdapter;
use pollux_schema::gemini::{GeminiGenerateContentRequest, GeminiResponseBody};
use pollux_thoughtsig_core::{SignatureSniffer, ThoughtSignatureEngine};
use std::sync::Arc;

const DEFAULT_TTL_SECS: u64 = 60 * 60;
const DEFAULT_MAX_CAPACITY: u64 = 200_000;

#[derive(Clone)]
pub struct GeminiThoughtSigService {
    engine: Arc<ThoughtSignatureEngine>,
}

impl GeminiThoughtSigService {
    pub fn new() -> Self {
        let engine = ThoughtSignatureEngine::new(DEFAULT_TTL_SECS, DEFAULT_MAX_CAPACITY);

        Self {
            engine: Arc::new(engine),
        }
    }

    pub fn new_stream_sniffer(&self) -> SignatureSniffer {
        SignatureSniffer::new(self.engine.clone())
    }

    pub fn patch_request(&self, request: &mut GeminiGenerateContentRequest) {
        patch_request(request, self.engine.as_ref())
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

        service.patch_request(&mut req);
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

        service.patch_request(&mut req);
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

        service.patch_request(&mut req);
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

        service.patch_request(&mut req);
        assert_eq!(
            req.contents[0].parts[0].thought_signature.as_deref(),
            Some("stream_sig_001")
        );
    }
}
