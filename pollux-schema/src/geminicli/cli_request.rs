use crate::gemini::GeminiGenerateContentRequest;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct GeminiCliRequestMeta {
    pub model: String,
    pub project: String,
}

impl GeminiCliRequestMeta {
    /// Build a Gemini CLI upstream envelope from runtime metadata and
    /// a typed Gemini `generateContent` request body.
    pub fn into_request(self, request: GeminiGenerateContentRequest) -> GeminiCliRequest {
        GeminiCliRequest {
            model: self.model,
            project: self.project,
            request,
        }
    }
}

/// Gemini CLI upstream request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCliRequest {
    pub model: String,
    pub project: String,
    pub request: GeminiGenerateContentRequest,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn into_request_fills_envelope() {
        let request: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": [{
                "role": "user",
                "parts": [{"text": "hello"}]
            }]
        }))
        .unwrap();

        let body = GeminiCliRequestMeta {
            model: "gemini-2.5-flash".to_string(),
            project: "project-1".to_string(),
        }
        .into_request(request);

        assert_eq!(body.model, "gemini-2.5-flash");
        assert_eq!(body.project, "project-1");
    }

    #[test]
    fn envelope_roundtrips() {
        let input = json!({
            "model": "gemini-2.5-pro",
            "project": "project-1",
            "request": {
                "contents": [{
                    "role": "user",
                    "parts": [{"text": "ping"}]
                }]
            }
        });

        let body: GeminiCliRequest = serde_json::from_value(input.clone()).unwrap();
        let output = serde_json::to_value(body).unwrap();
        assert_eq!(output, input);
    }
}
