use crate::config::GeminiCliResolvedConfig;
use crate::error::{GeminiCliError, GeminiCliErrorBody, IsRetryable};
use crate::providers::geminicli::{GeminiCliActorHandle, GeminiContext};
use crate::providers::policy::classify_upstream_error;
use backon::{ExponentialBuilder, Retryable};
use pollux_schema::{gemini::GeminiGenerateContentRequest, geminicli::GeminiCliRequestMeta};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

use super::api::GeminiApi;

pub struct GeminiClient {
    client: reqwest::Client,
    retry_policy: ExponentialBuilder,
}

impl GeminiClient {
    pub fn new(cfg: &GeminiCliResolvedConfig, client: reqwest::Client) -> Self {
        let retry_policy = ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(100))
            .with_max_delay(Duration::from_millis(300))
            .with_max_times(cfg.retry_max_times)
            .with_jitter();
        Self {
            client,
            retry_policy,
        }
    }

    pub async fn call_gemini_cli(
        &self,
        handle: &GeminiCliActorHandle,
        ctx: &GeminiContext,
        body: &GeminiGenerateContentRequest,
    ) -> Result<reqwest::Response, GeminiCliError> {
        let base_request = body.clone();
        let model = ctx.model.clone();
        let model_mask = ctx.model_mask;

        let handle = handle.clone();
        let client = self.client.clone();
        let stream = ctx.stream;
        let retry_policy_inner = self.retry_policy;

        let op = {
            let base_request = base_request.clone();
            move || {
                let handle = handle.clone();
                let client = client.clone();
                let base_request = base_request.clone();
                let model = model.clone();
                async move {
                    let start = Instant::now();
                    let assigned = handle
                        .get_credential(model_mask)
                        .await?
                        .ok_or(GeminiCliError::NoAvailableCredential)?;

                    let actor_took = start.elapsed();
                    info!(
                        channel = "geminicli",
                        lease.id = assigned.id,
                        lease.waited_us = actor_took.as_micros() as u64,
                        req.model = %model,
                        req.stream = stream,

                        "[GeminiCli] [ID: {}] [{:?}] Post responses -> {}",
                        assigned.id,
                        actor_took,
                        model.as_str()
                    );

                    let payload = GeminiCliRequestMeta {
                        model: model.clone(),
                        project: assigned.project_id.clone(),
                    }
                    .into_request(base_request.clone());

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

                        let (action, final_error) = classify_upstream_error(
                            resp,
                            |json: GeminiCliErrorBody| GeminiCliError::UpstreamMappedError {
                                status,
                                body: json,
                            },
                            |status, body| GeminiCliError::UpstreamFallbackError { status, body },
                        )
                        .await;

                        match &action {
                            crate::providers::ActionForError::RateLimit(duration) => {
                                handle
                                    .report_rate_limit(assigned.id, model_mask, *duration)
                                    .await;
                                info!(
                                    "Project: {}, rate limited, retry in {:?}",
                                    assigned.project_id, duration
                                );
                            }
                            crate::providers::ActionForError::Ban => {
                                handle.report_baned(assigned.id).await;
                                info!("Project: {}, banned", assigned.project_id);
                            }
                            crate::providers::ActionForError::ModelUnsupported => {
                                handle
                                    .report_model_unsupported(assigned.id, model_mask)
                                    .await;
                                info!("Project: {}, model unsupported", assigned.project_id);
                            }
                            crate::providers::ActionForError::Invalid => {
                                handle.report_invalid(assigned.id).await;
                                info!("Project: {}, invalid", assigned.project_id);
                            }
                            crate::providers::ActionForError::None => {}
                        }

                        match &final_error {
                            GeminiCliError::UpstreamMappedError { status, .. } => {
                                warn!(
                                    lease_id = assigned.id,
                                    model = %model,
                                    status = %status,
                                    action = ?action,
                                    "[GeminiCli] Upstream mapped error"
                                );
                            }
                            GeminiCliError::UpstreamFallbackError { status, .. } => {
                                warn!(
                                    lease_id = assigned.id,
                                    model = %model,
                                    status = %status,
                                    action = ?action,
                                    "[GeminiCli] Upstream fallback error"
                                );
                            }
                            GeminiCliError::Reqwest(error) => {
                                warn!(
                                    lease_id = assigned.id,
                                    model = %model,
                                    status = ?error.status(),
                                    action = ?action,
                                    "[GeminiCli] Upstream reqwest error"
                                );
                            }
                            _ => {
                                warn!(
                                    lease_id = assigned.id,
                                    model = %model,
                                    status = "N/A",
                                    action = ?action,
                                    "[GeminiCli] Upstream other error"
                                );
                            }
                        }

                        return Err(final_error);
                    }
                    Ok(resp)
                }
            }
        };

        op.retry(&self.retry_policy)
            .when(|err: &GeminiCliError| err.is_retryable())
            .notify(|err, dur: Duration| {
                error!(
                    "[GeminiCLI] Upstream Error {} retry after {:?}",
                    err.to_string(),
                    dur
                );
            })
            .await
    }
}
