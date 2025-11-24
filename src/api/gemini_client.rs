use crate::error::{GeminiError, IsRetryable, NexusError};
use crate::middleware::gemini_request::{GeminiContext, GeminiRequestBody};
use crate::router::NexusState;
use axum::http::StatusCode;
use backon::{ExponentialBuilder, Retryable};
use serde::Serialize;
use std::time::Duration;
use tracing::{error, info, warn};

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
            .with_max_times(3)
            .with_jitter();
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
            .when(|err: &NexusError| err.is_retryable())
            .notify(|err, dur: Duration| {
                error!(
                    "GeminiCLI Retrying Error {} with sleeping {:?}",
                    err.to_string(),
                    dur
                );
            })
            .await
    }
}
