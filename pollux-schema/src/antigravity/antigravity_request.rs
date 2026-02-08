//! Typed request schema for the Antigravity upstream envelope.
//!
//! Antigravity uses a wrapper payload around Gemini's generate-content request.

use crate::gemini::GeminiGenerateContentRequest;
use serde::{Deserialize, Serialize};

/// Runtime metadata needed to wrap a Gemini request into
/// Antigravity's upstream envelope.
#[derive(Debug, Clone)]
pub struct AntigravityRequestMeta {
    pub project: String,
    pub request_id: String,
    pub model: String,
}

/// Antigravity upstream request envelope.
///
/// All fields are required.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityRequestBody {
    pub project: String,
    pub request_id: String,
    pub request: GeminiGenerateContentRequest,
    pub model: String,
    pub user_agent: String,
    pub request_type: String,
}

impl AntigravityRequestBody {
    pub const USER_AGENT: &str = "antigravity";
    pub const REQUEST_TYPE: &str = "agent";
}

impl From<(GeminiGenerateContentRequest, AntigravityRequestMeta)> for AntigravityRequestBody {
    fn from((request, meta): (GeminiGenerateContentRequest, AntigravityRequestMeta)) -> Self {
        Self {
            project: meta.project,
            request_id: meta.request_id,
            request,
            model: meta.model,
            user_agent: Self::USER_AGENT.to_string(),
            request_type: Self::REQUEST_TYPE.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn full_envelope_roundtrips() {
        let input = json!({
            "project": "test-project",
            "requestId": "agent/1770489747018/b9acb5be-0d95-407e-a9cf-94315ff8a43e",
            "request": {
                "contents": [{
                    "role": "user",
                    "parts": [{"text": "hello"}]
                }]
            },
            "model": "claude-sonnet-4-5-thinking",
            "userAgent": "antigravity",
            "requestType": "agent"
        });

        let body: AntigravityRequestBody = serde_json::from_value(input.clone()).unwrap();
        let output = serde_json::to_value(&body).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn all_fields_are_required() {
        let err = serde_json::from_value::<AntigravityRequestBody>(json!({
            "project": "test-project",
            "request": {"contents": []},
            "model": "claude-sonnet-4-5-thinking",
            "userAgent": "antigravity",
            "requestType": "agent"
        }))
        .unwrap_err();

        assert!(err.to_string().contains("requestId"));
    }

    #[test]
    fn request_field_uses_gemini_generate_content_shape() {
        let err = serde_json::from_value::<AntigravityRequestBody>(json!({
            "project": "test-project",
            "requestId": "req-1",
            "request": {},
            "model": "claude-sonnet-4-5-thinking",
            "userAgent": "antigravity",
            "requestType": "agent"
        }))
        .unwrap_err();

        assert!(err.to_string().contains("contents"));
    }

    #[test]
    fn from_gemini_request_applies_fixed_fields() {
        let request = serde_json::from_value::<GeminiGenerateContentRequest>(json!({
            "contents": [{
                "role": "user",
                "parts": [{"text": "hello"}]
            }]
        }))
        .unwrap();

        let body = AntigravityRequestBody::from((
            request,
            AntigravityRequestMeta {
                project: "project-1".to_string(),
                request_id: "agent/1/00000000-0000-4000-8000-000000000000".to_string(),
                model: "claude-sonnet-4-5-thinking".to_string(),
            },
        ));

        assert_eq!(body.user_agent, "antigravity");
        assert_eq!(body.request_type, "agent");
        assert_eq!(body.project, "project-1");
        assert_eq!(body.model, "claude-sonnet-4-5-thinking");
    }
}
