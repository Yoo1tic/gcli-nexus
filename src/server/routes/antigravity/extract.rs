use crate::error::{GeminiCliError, GeminiErrorObject};
use crate::providers::antigravity::AntigravityContext;
use crate::server::router::PolluxState;
use crate::utils::logging::with_pretty_json_debug;
use axum::{
    Json, RequestExt,
    extract::{FromRequest, Path, Request},
    http::StatusCode,
};
use pollux_schema::gemini::GeminiGenerateContentRequest;
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
        let Json(body) = req
            .extract::<Json<GeminiGenerateContentRequest>, _>()
            .await?;

        with_pretty_json_debug(&body, |pretty_body| {
            debug!(
                channel = "antigravity",
                req.model = %model,
                req.stream = stream,
                req.path = %path,
                body = %pretty_body,
                "[Antigravity] Extracted normalized request body"
            );
        });

        let ctx = AntigravityContext {
            model,
            stream,
            path,
            model_mask,
        };
        Ok(AntigravityPreprocess(body, ctx))
    }
}
