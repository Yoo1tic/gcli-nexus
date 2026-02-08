use crate::config::CLAUDE_SYSTEM_PREAMBLE;
use crate::error::{GeminiCliError, GeminiErrorObject};
use crate::providers::antigravity::AntigravityContext;
use crate::server::router::PolluxState;
use axum::{
    Json, RequestExt,
    extract::{FromRequest, Path, Request},
    http::StatusCode,
};
use pollux_schema::gemini::GeminiGenerateContentRequest;
use serde_json::{Value, json};
use std::borrow::Borrow;
use tracing::{debug, warn};

pub struct AntigravityPreprocess(pub GeminiGenerateContentRequest, pub AntigravityContext);

impl<S> FromRequest<S> for AntigravityPreprocess
where
    S: Send + Sync + Borrow<PolluxState>,
{
    type Rejection = GeminiCliError;

    async fn from_request(mut req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Path(path) = req
            .extract_parts::<Path<String>>()
            .await
            .map_err(|rejection| GeminiCliError::RequestRejected {
                status: StatusCode::BAD_REQUEST,
                body: GeminiErrorObject::for_status(
                    StatusCode::BAD_REQUEST,
                    "INVALID_ARGUMENT",
                    "invalid path",
                ),
                debug_message: Some(rejection.to_string()),
            })?;

        // Determine model and optional rpc from the last path segment.
        let last_seg = path.split('/').next_back().map(|s| s.to_string());
        let Some(last_seg) = last_seg else {
            return Err(GeminiCliError::RequestRejected {
                status: StatusCode::BAD_REQUEST,
                body: GeminiErrorObject::for_status(
                    StatusCode::BAD_REQUEST,
                    "INVALID_ARGUMENT",
                    "model not found in path",
                ),
                debug_message: None,
            });
        };
        let model = if let Some((m, _r)) = last_seg.split_once(':') {
            m.to_string()
        } else {
            last_seg
        };

        let state = state.borrow();
        let is_allowed = state
            .providers
            .antigravity_cfg
            .model_list
            .iter()
            .any(|m| m == &model);
        if !is_allowed {
            warn!(
                "Rejected request for unsupported antigravity model: {}",
                model
            );
            let body = GeminiErrorObject::for_status(
                StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENT",
                format!("unsupported model: {model}"),
            );
            return Err(GeminiCliError::RequestRejected {
                status: StatusCode::BAD_REQUEST,
                body,
                debug_message: None,
            });
        }

        let Some(model_mask) = crate::model_catalog::mask(model.as_str()) else {
            warn!(
                "Rejected request for antigravity model not in global catalog: {}",
                model
            );
            let body = GeminiErrorObject::for_status(
                StatusCode::BAD_REQUEST,
                "INVALID_ARGUMENT",
                format!("unsupported model: {model}"),
            );
            return Err(GeminiCliError::RequestRejected {
                status: StatusCode::BAD_REQUEST,
                body,
                debug_message: None,
            });
        };

        let stream = path.contains("streamGenerateContent");
        let Json(mut body) = req
            .extract::<Json<GeminiGenerateContentRequest>, _>()
            .await?;

        if model.starts_with("claude") {
            ensure_claude_system_instruction(&mut body);
        }

        let ctx = AntigravityContext {
            model,
            stream,
            path,
            model_mask,
        };
        Ok(AntigravityPreprocess(body, ctx))
    }
}

/// Ensures the request body contains the required Antigravity system
/// instruction preamble for Claude models. If a `systemInstruction` already
/// exists and contains the preamble marker (`**Proactiveness**`), it is left
/// untouched. Otherwise the preamble is prepended (or created from scratch).
fn ensure_claude_system_instruction(body: &mut GeminiGenerateContentRequest) {
    let mut payload = match serde_json::to_value(&*body) {
        Ok(value) => value,
        Err(error) => {
            warn!(
                error = %error,
                "Failed to serialize antigravity request body for systemInstruction normalization"
            );
            return;
        }
    };

    let existing_text = payload
        .get("systemInstruction")
        .and_then(|si| si.get("parts"))
        .and_then(Value::as_array)
        .and_then(|parts| parts.first())
        .and_then(|part| part.get("text"))
        .and_then(Value::as_str);

    if let Some(text) = existing_text
        && text.to_ascii_lowercase().contains("**proactiveness**")
    {
        return;
    }

    let final_text = match existing_text {
        Some(text) if !text.is_empty() => format!("{}\n{}", CLAUDE_SYSTEM_PREAMBLE, text),
        _ => CLAUDE_SYSTEM_PREAMBLE.to_string(),
    };

    debug!(
        text_len = final_text.len(),
        "[Antigravity] Injecting Claude system instruction preamble"
    );

    let Some(obj) = payload.as_object_mut() else {
        warn!("Antigravity request payload is not a JSON object");
        return;
    };

    obj.insert(
        "systemInstruction".to_string(),
        json!({
            "parts": [{ "text": final_text }]
        }),
    );

    match serde_json::from_value(payload) {
        Ok(updated) => *body = updated,
        Err(error) => {
            warn!(
                error = %error,
                "Failed to deserialize normalized antigravity request body"
            );
        }
    }
}
