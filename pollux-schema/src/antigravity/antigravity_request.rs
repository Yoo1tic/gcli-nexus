//! Typed request schema for the Antigravity upstream envelope.
//!
//! Antigravity uses a wrapper payload around Gemini's generate-content request.

use crate::gemini::{Content, GeminiGenerateContentRequest, Part};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Runtime metadata needed to wrap a Gemini request into
/// Antigravity's upstream envelope.
#[derive(Debug, Clone)]
pub struct AntigravityRequestMeta {
    pub project: String,
    pub request_id: String,
    pub model: String,
}

impl AntigravityRequestMeta {
    /// Build an Antigravity upstream envelope from runtime metadata and
    /// a typed Gemini `generateContent` request body.
    pub fn into_request(self, request: GeminiGenerateContentRequest) -> AntigravityRequestBody {
        AntigravityRequestBody {
            project: self.project,
            request_id: self.request_id,
            request,
            model: self.model,
            user_agent: AntigravityRequestBody::USER_AGENT.to_string(),
            request_type: AntigravityRequestBody::REQUEST_TYPE.to_string(),
        }
    }
}

impl AntigravityRequestBody {
    /// Blindly prepend text to the embedded Gemini `systemInstruction`.
    ///
    /// This method does not perform marker/dedup checks. If an existing
    /// `systemInstruction` first text part exists, `prefix` is prepended with
    /// a newline; otherwise `prefix` becomes the full instruction.
    pub fn prepend_system_instruction(&mut self, prefix: &str) {
        let system_instruction = self.request.system_instruction_mut();
        let next_text = system_instruction
            .as_ref()
            .and_then(|content| content.parts.first().and_then(|part| part.text.as_deref()))
            .map(|text| format!("{prefix}\n{text}"))
            .unwrap_or_else(|| prefix.to_string());

        let next = Content {
            role: None,
            parts: vec![Part {
                text: Some(next_text),
                ..Part::default()
            }],
            extra: BTreeMap::new(),
        };

        *system_instruction = Some(next);
    }
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
    fn into_request_applies_fixed_fields() {
        let request = serde_json::from_value::<GeminiGenerateContentRequest>(json!({
            "contents": [{
                "role": "user",
                "parts": [{"text": "hello"}]
            }]
        }))
        .unwrap();

        let body = AntigravityRequestMeta {
            project: "project-1".to_string(),
            request_id: "agent/1/00000000-0000-4000-8000-000000000000".to_string(),
            model: "claude-sonnet-4-5-thinking".to_string(),
        }
        .into_request(request);

        assert_eq!(body.user_agent, "antigravity");
        assert_eq!(body.request_type, "agent");
        assert_eq!(body.project, "project-1");
        assert_eq!(body.model, "claude-sonnet-4-5-thinking");
    }

    #[test]
    fn prepend_system_instruction_sets_instruction_when_missing() {
        let request: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": []
        }))
        .unwrap();

        let mut body = AntigravityRequestMeta {
            project: "project-1".to_string(),
            request_id: "agent/1/00000000-0000-4000-8000-000000000000".to_string(),
            model: "claude-sonnet-4-5-thinking".to_string(),
        }
        .into_request(request);

        body.prepend_system_instruction("PREAMBLE");

        let si = body.request.system_instruction.as_ref().unwrap();
        assert!(si.role.is_none());
        assert_eq!(si.parts[0].text.as_deref(), Some("PREAMBLE"));
    }

    #[test]
    fn prepend_system_instruction_is_blind_and_can_duplicate() {
        let request: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": [],
            "systemInstruction": {"parts": [{"text": "PREAMBLE\nexisting"}]}
        }))
        .unwrap();

        let mut body = AntigravityRequestMeta {
            project: "project-1".to_string(),
            request_id: "agent/1/00000000-0000-4000-8000-000000000000".to_string(),
            model: "claude-sonnet-4-5-thinking".to_string(),
        }
        .into_request(request);

        body.prepend_system_instruction("PREAMBLE");

        let text = body
            .request
            .system_instruction
            .as_ref()
            .and_then(|si| si.parts.first())
            .and_then(|part| part.text.as_deref());
        assert_eq!(text, Some("PREAMBLE\nPREAMBLE\nexisting"));
    }
}
