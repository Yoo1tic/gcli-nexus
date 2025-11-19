use axum::{
    Json,
    body::Body,
    http::{
        HeaderValue, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING},
    },
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
};
use backon::{ExponentialBuilder, Retryable};
use eventsource_stream::{Event as UpstreamEvent, Eventsource};
use futures::{StreamExt, TryStreamExt};
use serde::Serialize;
use serde_json::json;
use std::{io, time::Duration};
use tracing::{error, info, warn};

use crate::error::{GeminiError, NexusError};
use crate::middleware::gemini_request::{GeminiContext, GeminiRequestBody};
use crate::router::NexusState;
use crate::types::cli::{cli_bytes_to_aistudio, cli_str_to_aistudio};

use super::gemini_api::GeminiApi;

pub struct GeminiClient {
    client: reqwest::Client,
    retry_policy: ExponentialBuilder,
}

#[derive(Clone, Serialize)]
struct CliPostFormatBody {
    model: String,
    project: String,
    request: GeminiRequestBody,
}

impl GeminiClient {
    pub fn new(client: reqwest::Client) -> Self {
        let retry_policy = ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(100))
            .with_max_delay(Duration::from_secs(1))
            .with_max_times(3);
        Self {
            client,
            retry_policy,
        }
    }

    pub async fn call_gemini_cli(
        &self,
        state: &NexusState,
        ctx: &GeminiContext,
        body: &GeminiRequestBody,
    ) -> Result<reqwest::Response, NexusError> {
        let base_payload = CliPostFormatBody {
            model: ctx.model.clone(),
            project: String::new(),
            request: body.clone(),
        };

        let handle = state.handle.clone();
        let client = self.client.clone();
        let stream = ctx.stream;
        let retry_policy_inner = self.retry_policy;

        let op = {
            let base_payload = base_payload.clone();
            move || {
                let handle = handle.clone();
                let client = client.clone();
                let base_payload = base_payload.clone();
                async move {
                    let assigned = handle
                        .get_credential(&ctx.model)
                        .await?
                        .ok_or(NexusError::NoAvailableCredential)?;

                    info!(
                        "Using credential ID: {} Project: {}",
                        assigned.id, assigned.project_id
                    );

                    let mut payload = base_payload.clone();
                    payload.project = assigned.project_id.clone();

                    let resp = GeminiApi::try_post_cli(
                        client.clone(),
                        assigned.access_token,
                        stream,
                        retry_policy_inner,
                        &payload,
                    )
                    .await?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let bytes = resp.bytes().await.map_err(NexusError::ReqwestError)?;

                        if let Ok(gemini_err) = serde_json::from_slice::<GeminiError>(&bytes) {
                            let http_code = gemini_err.error.code;
                            warn!(
                                project = %assigned.project_id,
                                error_struct = ?gemini_err,
                                "Upstream returned structured error"
                            );
                            match gemini_err.error.status.as_str() {
                                "RESOURCE_EXHAUSTED" => {
                                    let retry_secs = gemini_err.quota_reset_delay().unwrap_or(90);
                                    handle
                                        .report_rate_limit(
                                            assigned.id,
                                            &ctx.model,
                                            Duration::from_secs(retry_secs),
                                        )
                                        .await;
                                    info!(
                                        "Project: {}, rate limit (parsed, {}s)",
                                        assigned.project_id, retry_secs
                                    );
                                }
                                "UNAUTHENTICATED" => {
                                    handle.report_invalid(assigned.id).await;
                                    info!("Project: {}, invalid (parsed)", assigned.project_id);
                                }

                                "PERMISSION_DENIED" if http_code == 403 => {
                                    handle.report_baned(assigned.id).await;
                                    info!("Project: {}, banned (parsed)", assigned.project_id);
                                }

                                _ if http_code == 401 => {
                                    handle.report_invalid(assigned.id).await;
                                }
                                _ if http_code == 429 => {
                                    handle
                                        .report_rate_limit(
                                            assigned.id,
                                            &ctx.model,
                                            Duration::from_secs(60),
                                        )
                                        .await;
                                }
                                _ => {}
                            }

                            return Err(NexusError::GeminiServerError(gemini_err));
                        } else {
                            let raw_body = String::from_utf8_lossy(&bytes);

                            match status {
                                StatusCode::TOO_MANY_REQUESTS => {
                                    handle
                                        .report_rate_limit(
                                            assigned.id,
                                            &ctx.model,
                                            Duration::from_secs(60),
                                        )
                                        .await;
                                    warn!(
                                        "Project: {}, 429 Rate limit (Fallback). Body: {:.100}...",
                                        assigned.project_id, raw_body
                                    );
                                }
                                StatusCode::FORBIDDEN => {
                                    warn!(
                                        "Project: {}, 403 Forbidden (Raw/WAF), preserving credential. Body: {:.100}...",
                                        assigned.project_id, raw_body
                                    );
                                }
                                _ => {
                                    warn!(
                                        "Upstream non-JSON error. Status: {}, Body: {:.100}",
                                        status, raw_body
                                    );
                                }
                            }

                            return Err(NexusError::UpstreamStatus(status));
                        }
                    }
                    Ok(resp)
                }
            }
        };

        op.retry(&self.retry_policy)
            .when(|err: &NexusError| match err {
                NexusError::GeminiServerError(e) => {
                    let status = e.error.status.as_str();
                    matches!(
                        status,
                        "RESOURCE_EXHAUSTED" | "UNAUTHENTICATED" | "PERMISSION_DENIED"
                    )
                }

                NexusError::ReqwestError(_) => true,
                _ => false,
            })
            .await
    }

    pub async fn build_json_response(upstream_resp: reqwest::Response) -> Response {
        let status = upstream_resp.status();
        let original_headers = upstream_resp.headers().clone();
        match upstream_resp.bytes().await {
            Ok(bytes) => {
                let converted = convert_cli_envelope_bytes(&bytes);
                let content_len = converted.len();
                let mut response = Response::builder()
                    .status(status)
                    .body(Body::from(converted))
                    .unwrap_or_else(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error":"build response failed"})),
                        )
                            .into_response()
                    });

                {
                    let headers_mut = response.headers_mut();
                    *headers_mut = original_headers;
                    headers_mut.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                    headers_mut.remove(CONTENT_LENGTH);
                    headers_mut.remove(TRANSFER_ENCODING);
                    if let Ok(value) = HeaderValue::from_str(&content_len.to_string()) {
                        headers_mut.insert(CONTENT_LENGTH, value);
                    }
                }

                response
            }
            Err(e) => {
                error!(error = %e, "failed to read upstream response body");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": "failed to read upstream response" })),
                )
                    .into_response()
            }
        }
    }

    pub fn build_stream_response(upstream_resp: reqwest::Response) -> Response {
        let status = upstream_resp.status();
        let original_headers = upstream_resp.headers().clone();
        let sse_stream = upstream_resp
            .bytes_stream()
            .map_err(io::Error::other)
            .eventsource()
            .map(|result| result.map_err(io::Error::other))
            .filter_map(|result| async {
                match result {
                    Ok(event) => convert_upstream_event(event).map(Ok),
                    Err(err) => Some(Err(err)),
                }
            });

        let mut response = Sse::new(sse_stream).into_response();
        *response.status_mut() = status;
        *response.headers_mut() = original_headers;
        response
    }
}

fn convert_cli_envelope_bytes(body: &[u8]) -> Vec<u8> {
    match cli_bytes_to_aistudio(body) {
        Ok(resp) => match serde_json::to_vec(&resp) {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!(error = %e, "failed to serialize converted CLI payload");
                body.to_vec()
            }
        },
        Err(e) => {
            warn!(error = %e, "failed to convert CLI response body");
            body.to_vec()
        }
    }
}

fn convert_upstream_event(event: UpstreamEvent) -> Option<Event> {
    let payload = convert_cli_sse_payload(&event.data)?;
    let mut axum_event = Event::default().data(payload);
    if !event.event.is_empty() && event.event != "message" {
        axum_event = axum_event.event(event.event);
    }
    if !event.id.is_empty() {
        axum_event = axum_event.id(event.id);
    }
    if let Some(retry) = event.retry {
        axum_event = axum_event.retry(retry);
    }
    Some(axum_event)
}

fn convert_cli_sse_payload(payload: &str) -> Option<String> {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return None;
    }
    match cli_str_to_aistudio(trimmed).and_then(|resp| serde_json::to_string(&resp)) {
        Ok(converted) => Some(converted),
        Err(e) => {
            warn!(error = %e, "failed to parse CLI SSE payload as JSON");
            Some(trimmed.to_string())
        }
    }
}
