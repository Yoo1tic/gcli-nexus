use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// `generationConfig` object.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_temperature"
    )]
    pub temperature: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,

    /// Keep `thinkingConfig` as raw value for transparent pass-through.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_config: Option<Value>,

    /// Config for image generation features.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_config: Option<Value>,

    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl GenerationConfig {
    /// Mutable access to `generationConfig.thinkingConfig` field.
    pub fn thinking_config_mut(&mut self) -> &mut Option<Value> {
        &mut self.thinking_config
    }
}

fn deserialize_temperature<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<f64>::deserialize(deserializer)?;
    Ok(raw.map(|value| value.clamp(0.0, 2.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn temperature_is_clamped_into_supported_range() {
        let high: GenerationConfig = serde_json::from_value(json!({"temperature": 9.9})).unwrap();
        let low: GenerationConfig = serde_json::from_value(json!({"temperature": -1.0})).unwrap();

        assert_eq!(high.temperature, Some(2.0));
        assert_eq!(low.temperature, Some(0.0));
    }

    #[test]
    fn generation_config_extra_captures_unknown_fields() {
        let input = json!({
            "temperature": 1.0,
            "candidateCount": 2,
            "responseSchema": {"type": "object"},
            "responseJsonSchema": {"type": "object", "properties": {}},
            "responseModalities": ["TEXT"],
            "imageConfig": {
                "aspectRatio": "16:9",
                "imageSize": "2K"
            },
            "newFutureField": true,
            "thinkingConfig": {
                "thinkingLevel": "high",
                "thinkingBudget": 1024
            }
        });

        let gc: GenerationConfig = serde_json::from_value(input).unwrap();
        assert_eq!(gc.temperature, Some(1.0));
        assert_eq!(gc.extra.get("candidateCount"), Some(&json!(2)));
        assert_eq!(
            gc.extra.get("responseSchema"),
            Some(&json!({"type": "object"}))
        );
        assert_eq!(
            gc.extra.get("responseJsonSchema"),
            Some(&json!({"type": "object", "properties": {}}))
        );
        assert_eq!(gc.extra.get("responseModalities"), Some(&json!(["TEXT"])));
        assert_eq!(
            gc.image_config,
            Some(json!({
                "aspectRatio": "16:9",
                "imageSize": "2K"
            }))
        );
        assert_eq!(gc.extra.get("newFutureField"), Some(&json!(true)));
        assert_eq!(
            gc.thinking_config,
            Some(json!({
                "thinkingLevel": "high",
                "thinkingBudget": 1024
            }))
        );
    }

    #[test]
    fn thinking_config_roundtrips_as_raw_value() {
        let input = json!({
            "thinkingConfig": {
                "thinkingLevel": "high",
                "someVendorField": 1
            }
        });

        let gc: GenerationConfig = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(serde_json::to_value(&gc).unwrap(), input);
    }
}
