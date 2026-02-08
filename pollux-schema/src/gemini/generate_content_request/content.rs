use serde::{Deserialize, Serialize, de::Error};
use serde_json::Value;
use std::collections::BTreeMap;

/// A single conversation turn or system instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    /// Usually `"user"` or `"model"`. Absent for `systemInstruction`.
    ///
    /// Kept as raw string for transparent pass-through.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    /// Ordered parts that constitute a single message.
    #[serde(deserialize_with = "deserialize_parts")]
    pub parts: Vec<Part>,

    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One atomic piece of content inside a `Content` turn.
///
/// `text` is the most common variant; other part types (inlineData,
/// functionCall, functionResponse, …) are explicitly modeled, while
/// unrecognized fields are preserved in `extra`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    /// Optional model-thought marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought: Option<bool>,

    /// Opaque reusable thought signature (base64 string).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,

    /// Optional custom part metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_metadata: Option<Value>,

    /// Inline text data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// Inline media bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_data: Option<Value>,

    /// Function call produced by model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<Value>,

    /// Function response used as context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_response: Option<Value>,

    /// URI-based file data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_data: Option<Value>,

    /// Executable code block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_code: Option<Value>,

    /// Code execution result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_execution_result: Option<Value>,

    /// Optional video metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_metadata: Option<Value>,

    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn deserialize_parts<'de, D>(deserializer: D) -> Result<Vec<Part>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let parts = Vec::<Part>::deserialize(deserializer)?;

    for (index, part) in parts.iter().enumerate() {
        let data_fields_count = usize::from(part.text.is_some())
            + usize::from(part.inline_data.is_some())
            + usize::from(part.function_call.is_some())
            + usize::from(part.function_response.is_some())
            + usize::from(part.file_data.is_some())
            + usize::from(part.executable_code.is_some())
            + usize::from(part.code_execution_result.is_some());

        if data_fields_count > 1 {
            return Err(D::Error::custom(format!(
                "parts[{index}] must contain at most one data field among text, inlineData, functionCall, functionResponse, fileData, executableCode, codeExecutionResult"
            )));
        }
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn role_is_transparent_string() {
        let input = json!({
            "role": "SYSTEM",
            "parts": [{"text": "x"}]
        });

        let content: Content = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(content.role.as_deref(), Some("SYSTEM"));
        assert_eq!(serde_json::to_value(&content).unwrap(), input);
    }

    #[test]
    fn content_parts_is_required() {
        let err = serde_json::from_value::<Content>(json!({
            "role": "user"
        }))
        .unwrap_err();
        assert!(err.to_string().contains("parts"));
    }

    #[test]
    fn part_rejects_multiple_data_fields() {
        let err = serde_json::from_value::<Content>(json!({
            "role": "user",
            "parts": [{
                "text": "hello",
                "inlineData": {"mimeType": "text/plain", "data": "aGVsbG8="}
            }]
        }))
        .unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("parts[0]"));
        assert!(err_msg.contains("at most one data field"));
    }

    #[test]
    fn inline_data_field_maps() {
        let content: Content = serde_json::from_value(json!({
            "role": "user",
            "parts": [{
                "inlineData": {
                    "mimeType": "image/png",
                    "data": "abc123"
                }
            }]
        }))
        .unwrap();

        let part = &content.parts[0];
        assert!(part.text.is_none());
        assert!(part.inline_data.is_some());
    }

    #[test]
    fn function_call_and_response_parts_preserved() {
        let input = json!([
            {
                "role": "model",
                "parts": [{
                    "functionCall": {
                        "name": "get_weather",
                        "args": {"city": "London"}
                    }
                }]
            },
            {
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": "get_weather",
                        "response": {"temperature": 15}
                    }
                }]
            }
        ]);

        let contents: Vec<Content> = serde_json::from_value(input.clone()).unwrap();
        let output = serde_json::to_value(&contents).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn part_known_fields_roundtrip() {
        let input = json!({
            "role": "user",
            "parts": [{
                "thought": true,
                "thoughtSignature": "c2ln",
                "partMetadata": {"source": "file.md"},
                "fileData": {"mimeType": "text/plain", "fileUri": "gs://a/b"},
                "videoMetadata": {"startOffset": "1s", "endOffset": "2s"}
            }]
        });

        let content: Content = serde_json::from_value(input.clone()).unwrap();
        let part = &content.parts[0];
        assert_eq!(part.thought, Some(true));
        assert_eq!(part.thought_signature.as_deref(), Some("c2ln"));
        assert!(part.part_metadata.is_some());
        assert!(part.file_data.is_some());
        assert!(part.video_metadata.is_some());

        let output = serde_json::to_value(&content).unwrap();
        assert_eq!(output, input);
    }
}
