use crate::providers::geminicli::{GeminiContext, model_mask};
use crate::{error::GeminiCliError, error::GeminiErrorObject};
use axum::{
    Json, RequestExt,
    extract::{FromRequest, Path, Request},
    http::StatusCode,
};
use pollux_schema::gemini::GeminiGenerateContentRequest;
use tracing::warn;

pub struct GeminiPreprocess(pub GeminiGenerateContentRequest, pub GeminiContext);

impl<S> FromRequest<S> for GeminiPreprocess
where
    S: Send + Sync,
{
    type Rejection = GeminiCliError;

    async fn from_request(mut req: Request, _state: &S) -> Result<Self, Self::Rejection> {
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

        // Determine model and optional rpc from the last path segment
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

        let Some(model_mask) = model_mask(model.as_str()) else {
            warn!("Rejected request for unsupported model: {}", model);
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

        let Json(body) = Json::<GeminiGenerateContentRequest>::from_request(req, &()).await?;

        let ctx = GeminiContext {
            model,
            stream,
            path,
            model_mask,
        };
        Ok(GeminiPreprocess(body, ctx))
    }
}
