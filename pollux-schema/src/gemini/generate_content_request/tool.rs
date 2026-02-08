use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// `tools[]` object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    /// Function declarations available for model function calling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_declarations: Option<Vec<FunctionDeclaration>>,

    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Structured declaration for a callable function tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDeclaration {
    /// Function name.
    pub name: String,

    /// Brief function description.
    pub description: String,

    /// Optional function behavior enum.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,

    /// OpenAPI-style parameters schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,

    /// JSON Schema parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters_json_schema: Option<Value>,

    /// OpenAPI-style response schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,

    /// JSON Schema response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_json_schema: Option<Value>,

    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_roundtrip_with_known_and_unknown_fields() {
        let input = json!([
            {
                "functionDeclarations": [
                    {
                        "name": "run_command",
                        "description": "Run a shell command",
                        "parametersJsonSchema": {
                            "type": "object",
                            "properties": {
                                "cmd": {"type": "string"}
                            },
                            "required": ["cmd"]
                        }
                    }
                ]
            },
            {"codeExecution": {"enabled": true}}
        ]);

        let tools: Vec<Tool> = serde_json::from_value(input.clone()).unwrap();

        let declarations = tools[0].function_declarations.as_ref().unwrap();
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].name, "run_command");
        assert_eq!(declarations[0].description, "Run a shell command");
        assert!(declarations[0].parameters_json_schema.is_some());
        assert!(declarations[0].parameters.is_none());

        assert!(tools[1].function_declarations.is_none());
        assert!(tools[1].extra.contains_key("codeExecution"));

        assert_eq!(serde_json::to_value(&tools).unwrap(), input);
    }

    #[test]
    fn transparent_gateway_preserves_upstream_validation_cases() {
        let input = json!([
            {
                "functionDeclarations": [
                    {
                        "name": "bad name with space",
                        "description": "desc",
                        "parameters": {"type": "object"},
                        "parametersJsonSchema": {"type": "object"},
                        "response": {"type": "object"},
                        "responseJsonSchema": {"type": "object"}
                    }
                ]
            }
        ]);

        let tools: Vec<Tool> = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(serde_json::to_value(&tools).unwrap(), input);
    }
}
