use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// `toolConfig` object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    /// Function-calling configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_calling_config: Option<Value>,

    /// Retrieval configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_config: Option<Value>,

    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_config_roundtrip_with_known_fields() {
        let input = json!({
            "functionCallingConfig": {"mode": "AUTO"},
            "retrievalConfig": {"latencyBudgetMs": 300}
        });
        let tool_cfg: ToolConfig = serde_json::from_value(input.clone()).unwrap();

        assert_eq!(
            tool_cfg.function_calling_config,
            Some(json!({"mode": "AUTO"}))
        );
        assert_eq!(
            tool_cfg.retrieval_config,
            Some(json!({"latencyBudgetMs": 300}))
        );
        assert_eq!(serde_json::to_value(&tool_cfg).unwrap(), input);
    }

    #[test]
    fn tool_config_roundtrip_with_unknown_fields() {
        let input = json!({
            "functionCallingConfig": {"mode": "AUTO"},
            "someFutureField": true
        });
        let tool_cfg: ToolConfig = serde_json::from_value(input.clone()).unwrap();

        assert_eq!(tool_cfg.extra.get("someFutureField"), Some(&json!(true)));
        assert_eq!(serde_json::to_value(&tool_cfg).unwrap(), input);
    }
}
