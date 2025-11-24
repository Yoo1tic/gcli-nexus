use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Finish reasons returned by AiStudio.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum FinishReason {
    FINISH_REASON_UNSPECIFIED,
    STOP,
    MAX_TOKENS,
    SAFETY,
    RECITATION,
    LANGUAGE,
    OTHER,
    BLOCKLIST,
    PROHIBITED_CONTENT,
    SPII,
    MALFORMED_FUNCTION_CALL,
    IMAGE_SAFETY,
}

/// Chat content payload for candidates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct Chat {
    pub role: String,
    pub parts: Vec<Value>,
}

/// AiStudio candidate wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct Candidate {
    pub content: Chat,
    pub finishReason: Option<FinishReason>,
}

/// Final AiStudio-compatible response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct GeminiResponse {
    pub candidates: Vec<Candidate>,
    pub usageMetadata: Value,
    pub modelVersion: String,
    pub promptFeedback: Option<Value>,
}
