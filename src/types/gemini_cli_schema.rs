use serde::Deserialize;
use serde_json::Value;

use super::gemini_native_schema::{Candidate as AiCandidate, Chat, FinishReason, GeminiResponse};

/// Generic CLI envelope wrapper.
#[derive(Debug, Deserialize)]
pub struct CliEnvelope {
    pub response: CliResponse,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct CliResponse {
    pub candidates: Vec<CliCandidate>,
    pub usageMetadata: Value,
    pub modelVersion: String,
    #[serde(default)]
    pub promptFeedback: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct CliCandidate {
    pub content: Chat,
    #[serde(default)]
    pub finishReason: Option<FinishReason>,
}

impl From<CliResponse> for GeminiResponse {
    fn from(value: CliResponse) -> Self {
        let candidates = value
            .candidates
            .into_iter()
            .map(CliCandidate::into_native)
            .collect();
        GeminiResponse {
            candidates,
            usageMetadata: value.usageMetadata,
            modelVersion: value.modelVersion,
            promptFeedback: value.promptFeedback,
        }
    }
}

impl From<CliEnvelope> for GeminiResponse {
    fn from(envelope: CliEnvelope) -> Self {
        envelope.response.into()
    }
}

impl CliCandidate {
    fn into_native(self) -> AiCandidate {
        AiCandidate {
            content: self.content,
            finishReason: self.finishReason,
        }
    }
}
