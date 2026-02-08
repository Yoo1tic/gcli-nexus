use super::{Content, Part};
use serde::Deserialize;
use std::collections::BTreeMap;

pub fn deserialize_system_instruction<'de, D>(deserializer: D) -> Result<Option<Content>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Some(content) = Option::<Content>::deserialize(deserializer)? else {
        return Ok(None);
    };

    let merged_text = content
        .parts
        .into_iter()
        .filter_map(|part| part.text.filter(|text| !text.trim().is_empty()))
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok((!merged_text.is_empty()).then(|| Content {
        role: None,
        parts: vec![Part {
            text: Some(merged_text),
            ..Default::default()
        }],
        extra: BTreeMap::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::IntoDeserializer;
    use serde_json::{Value, json};

    fn run(value: Value) -> Option<Content> {
        deserialize_system_instruction(value.into_deserializer()).unwrap()
    }

    #[test]
    fn system_instruction_without_role() {
        let value = json!({
            "parts": [{"text": "you are a coding assistant"}]
        });

        let si = run(value).unwrap();
        assert!(si.role.is_none());
        assert_eq!(
            si.parts[0].text.as_deref(),
            Some("you are a coding assistant")
        );
    }

    #[test]
    fn system_instruction_role_is_dropped_and_text_only_normalized() {
        let value = json!({
            "role": "user",
            "parts": [
                {"text": "be precise"},
                {"inlineData": {"mimeType": "image/png", "data": "abc"}}
            ],
            "someFutureField": true
        });

        let si = run(value).unwrap();
        assert!(si.role.is_none());
        assert_eq!(si.parts.len(), 1);
        assert_eq!(si.parts[0].text.as_deref(), Some("be precise"));
        assert!(si.parts[0].extra.is_empty());
        assert!(si.extra.is_empty());
    }

    #[test]
    fn system_instruction_string_form_rejected() {
        let value = json!("be concise");
        assert!(deserialize_system_instruction(value.into_deserializer()).is_err());
    }

    #[test]
    fn system_instruction_multiple_text_parts_are_merged() {
        let value = json!({
            "parts": [
                {"text": "be"},
                {"text": "concise"}
            ]
        });

        let si = run(value).unwrap();
        assert_eq!(si.parts.len(), 1);
        assert_eq!(si.parts[0].text.as_deref(), Some("be\n\nconcise"));
    }

    #[test]
    fn system_instruction_without_text_becomes_none() {
        let value = json!({
            "parts": [{"inlineData": {"mimeType": "image/png", "data": "abc"}}]
        });

        assert!(run(value).is_none());
    }

    #[test]
    fn system_instruction_whitespace_only_text_becomes_none() {
        let value = json!({
            "parts": [
                {"text": "   "},
                {"text": "\n\t"}
            ]
        });

        assert!(run(value).is_none());
    }

    #[test]
    fn system_instruction_non_empty_text_with_surrounding_whitespace_is_kept() {
        let value = json!({
            "parts": [
                {"text": "  keep me  "}
            ]
        });

        let si = run(value).unwrap();
        assert_eq!(si.parts.len(), 1);
        assert_eq!(si.parts[0].text.as_deref(), Some("  keep me  "));
    }
}
