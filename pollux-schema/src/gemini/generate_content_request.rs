//! Typed Gemini v1beta request schema for Gemini generate-content endpoints.
//!
//! Instead of passing through raw `serde_json::Value`, we parse the incoming
//! Gemini native request into properly typed structs. This gives us:
//! - Compile-time access to known fields (e.g. `systemInstruction` for Claude
//!   preamble injection).
//! - Forward compatibility via `extra` catch-all maps at every level.
//! - Validation-friendly request shape (e.g. required `contents`).

mod content;
mod generation;
mod system_instruction;
mod tool;
mod tool_config;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use content::{Content, Part};
use generation::GenerationConfig;
use system_instruction::deserialize_system_instruction;
pub use tool::Tool;
pub use tool_config::ToolConfig;

/// Gemini `generateContent` / `streamGenerateContent` request body.
///
/// Reference: <https://ai.google.dev/gemini-api/docs/text-generation>
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiGenerateContentRequest {
    /// Required conversation turns.
    pub contents: Vec<Content>,

    /// System-level instruction. Structured identically to a `Content` but
    /// typically contains only a single text part with no `role`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_system_instruction"
    )]
    pub system_instruction: Option<Content>,

    /// Generation parameters (temperature, topP, maxOutputTokens, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GenerationConfig>,

    /// Tool declarations (function calling, code execution, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,

    /// Tool-calling configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<ToolConfig>,

    /// Catch-all for future/optional unknown fields, including
    /// `safetySettings` and `cachedContent`.
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn minimal_request_deserializes_with_defaults() {
        let req: GeminiGenerateContentRequest =
            serde_json::from_value(json!({"contents": []})).unwrap();
        assert!(req.contents.is_empty());
        assert!(req.system_instruction.is_none());
        assert!(req.generation_config.is_none());
        assert!(req.tools.is_none());
        assert!(req.extra.is_empty());
    }

    #[test]
    fn missing_contents_rejected() {
        let err = serde_json::from_value::<GeminiGenerateContentRequest>(json!({})).unwrap_err();
        assert!(err.to_string().contains("contents"));
    }

    #[test]
    fn full_request_roundtrips() {
        let input = json!({
            "contents": [{
                "role": "user",
                "parts": [{"text": "hello"}]
            }],
            "systemInstruction": {
                "parts": [{"text": "be helpful"}]
            },
            "generationConfig": {
                "temperature": 0.7,
                "topP": 0.9,
                "topK": 40,
                "maxOutputTokens": 1024,
                "stopSequences": ["END"],
                "responseMimeType": "text/plain",
                "thinkingConfig": {
                    "thinkingBudget": 2048
                }
            },
            "tools": [{"functionDeclarations": []}],
            "toolConfig": {"functionCallingConfig": {"mode": "AUTO"}},
            "safetySettings": [{"category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_NONE"}]
        });

        let req: GeminiGenerateContentRequest = serde_json::from_value(input.clone()).unwrap();

        assert_eq!(req.contents.len(), 1);
        assert_eq!(req.contents[0].role.as_deref(), Some("user"));
        assert_eq!(req.contents[0].parts[0].text.as_deref(), Some("hello"));
        assert_eq!(
            req.system_instruction.as_ref().unwrap().parts[0]
                .text
                .as_deref(),
            Some("be helpful")
        );

        let gc = req.generation_config.as_ref().unwrap();
        assert_eq!(gc.temperature, Some(0.7));
        assert_eq!(gc.top_p, Some(0.9));
        assert_eq!(gc.max_output_tokens, Some(1024));
        assert_eq!(gc.extra.get("stopSequences"), Some(&json!(["END"])));
        assert_eq!(gc.extra.get("responseMimeType"), Some(&json!("text/plain")));
        assert_eq!(
            gc.thinking_config,
            Some(json!({
                "thinkingBudget": 2048
            }))
        );

        // Roundtrip: serialize back and compare
        let output = serde_json::to_value(&req).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn unknown_fields_preserved_in_extra() {
        let input = json!({
            "contents": [{"parts": [{"text": "ping"}]}],
            "cachedContent": "projects/foo/cachedContents/bar",
            "someNewField": 42
        });

        let req: GeminiGenerateContentRequest = serde_json::from_value(input).unwrap();
        assert_eq!(
            req.extra.get("cachedContent"),
            Some(&json!("projects/foo/cachedContents/bar"))
        );
        assert_eq!(req.extra.get("someNewField"), Some(&json!(42)));
    }

    #[test]
    fn multi_turn_contents() {
        let input = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "What is Rust?"}]},
                {"role": "model", "parts": [{"text": "Rust is a systems programming language."}]},
                {"role": "user", "parts": [{"text": "Tell me more."}]}
            ]
        });

        let req: GeminiGenerateContentRequest = serde_json::from_value(input).unwrap();
        assert_eq!(req.contents.len(), 3);
        assert_eq!(req.contents[0].role.as_deref(), Some("user"));
        assert_eq!(req.contents[1].role.as_deref(), Some("model"));
        assert_eq!(req.contents[2].role.as_deref(), Some("user"));
    }

    #[test]
    fn multi_part_content() {
        let input = json!({
            "contents": [{
                "role": "user",
                "parts": [
                    {"text": "describe this image"},
                    {"inlineData": {"mimeType": "image/jpeg", "data": "base64data"}}
                ]
            }]
        });

        let req: GeminiGenerateContentRequest = serde_json::from_value(input).unwrap();
        assert_eq!(req.contents[0].parts.len(), 2);
        assert_eq!(
            req.contents[0].parts[0].text.as_deref(),
            Some("describe this image")
        );
        assert!(req.contents[0].parts[1].text.is_none());
        assert!(req.contents[0].parts[1].inline_data.is_some());
    }

    /// Mirrors the real Antigravity IDE request captured in antiREV/.
    #[test]
    fn real_antigravity_ide_request_roundtrips() {
        let input = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "user info block"}]},
                {"role": "user", "parts": [{"text": "artifact guidelines"}]},
                {"role": "user", "parts": [{"text": "step 0: user request"}]},
            ],
            "systemInstruction": {
                "role": "user",
                "parts": [{"text": "You are Antigravity..."}]
            },
            "tools": [
                {"functionDeclarations": [
                    {"name": "run_command", "description": "run a command", "parameters": {"type": "OBJECT"}}
                ]},
                {"functionDeclarations": [
                    {"name": "view_file", "description": "view a file", "parameters": {"type": "OBJECT"}}
                ]}
            ],
            "toolConfig": {
                "functionCallingConfig": {"mode": "VALIDATED"}
            },
            "generationConfig": {
                "temperature": 0.4,
                "topP": 1.0,
                "topK": 50,
                "candidateCount": 1,
                "maxOutputTokens": 16384,
                "stopSequences": ["<|user|>", "<|bot|>", "<|endoftext|>"],
                "thinkingConfig": {
                    "includeThoughts": true,
                    "thinkingBudget": 1024
                }
            },
            "sessionId": "-3750763034362895579"
        });

        let req: GeminiGenerateContentRequest = serde_json::from_value(input.clone()).unwrap();

        // Verify typed field access
        assert_eq!(req.contents.len(), 3);
        let si = req.system_instruction.as_ref().unwrap();
        assert!(si.role.is_none());

        let gc = req.generation_config.as_ref().unwrap();
        assert_eq!(gc.temperature, Some(0.4));
        assert_eq!(gc.top_p, Some(1.0));
        assert_eq!(gc.top_k, Some(50));
        assert_eq!(gc.max_output_tokens, Some(16384));
        assert_eq!(gc.extra.get("candidateCount"), Some(&json!(1)));
        assert_eq!(
            gc.extra.get("stopSequences"),
            Some(&json!(["<|user|>", "<|bot|>", "<|endoftext|>"]))
        );
        assert_eq!(
            gc.thinking_config,
            Some(json!({
                "includeThoughts": true,
                "thinkingBudget": 1024
            }))
        );

        // sessionId lands in top-level extra
        assert_eq!(
            req.extra.get("sessionId"),
            Some(&json!("-3750763034362895579"))
        );

        // tools and toolConfig preserved
        assert_eq!(req.tools.as_ref().unwrap().len(), 2);
        assert!(req.tool_config.is_some());

        // Roundtrip fidelity
        let output = serde_json::to_value(&req).unwrap();
        let mut expected = input;
        if let Some(obj) = expected
            .get_mut("systemInstruction")
            .and_then(Value::as_object_mut)
        {
            obj.remove("role");
        }
        assert_eq!(output, expected);
    }
}
