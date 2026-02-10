use super::Content;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Gemini v1beta schema types.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct GeminiResponseBody {
    #[serde(default)]
    pub candidates: Vec<Candidate>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub promptFeedback: Option<Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub usageMetadata: Option<Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub modelVersion: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub responseId: Option<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,

    #[serde(rename = "finishReason", skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
