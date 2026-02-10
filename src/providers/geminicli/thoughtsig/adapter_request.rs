use pollux_schema::gemini::GeminiGenerateContentRequest;
use pollux_thoughtsig_core::{FillAction, FillDecision};
use serde_json::Value;

pub(super) struct RequestPatchTarget {
    pub content_idx: usize,
    pub part_idx: usize,
    pub key_input: Option<Value>,
    pub existing_signature: Option<String>,
}

pub(super) fn collect_request_patch_targets(
    req: &GeminiGenerateContentRequest,
) -> Vec<RequestPatchTarget> {
    let mut targets = Vec::new();

    for (content_idx, content) in req.contents.iter().enumerate() {
        if content.role.as_deref() != Some("model") {
            continue;
        }

        for (part_idx, part) in content.parts.iter().enumerate() {
            if let Some(function_call) = part.function_call.as_ref() {
                targets.push(RequestPatchTarget {
                    content_idx,
                    part_idx,
                    key_input: Some(function_call.clone()),
                    existing_signature: part.thought_signature.clone(),
                });
                continue;
            }

            if part.thought == Some(true) {
                targets.push(RequestPatchTarget {
                    content_idx,
                    part_idx,
                    key_input: part.text.clone().map(Value::String),
                    existing_signature: part.thought_signature.clone(),
                });
            }
        }
    }

    targets
}

pub(super) fn apply_request_fill_decisions(
    req: &mut GeminiGenerateContentRequest,
    targets: &[RequestPatchTarget],
    decisions: &[FillDecision],
    dummy_signature: &str,
) {
    for (target, decision) in targets.iter().zip(decisions.iter()) {
        let Some(content) = req.contents.get_mut(target.content_idx) else {
            continue;
        };
        let Some(part) = content.parts.get_mut(target.part_idx) else {
            continue;
        };

        match &decision.action {
            FillAction::Keep => {}
            FillAction::UseCached(signature) => {
                part.thought_signature = Some(signature.to_string());
            }
            FillAction::UseDummy => {
                part.thought_signature = Some(dummy_signature.to_string());
            }
        }
    }
}
