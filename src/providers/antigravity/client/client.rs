use crate::config::AntigravityResolvedConfig;
use crate::error::{GeminiCliErrorBody, IsRetryable, PolluxError};
use crate::providers::antigravity::AntigravityActorHandle;
use crate::providers::policy::classify_upstream_error;
use backon::ExponentialBuilder;
use backon::Retryable;
use pollux_schema::antigravity::{AntigravityRequestBody, AntigravityRequestMeta};
use pollux_schema::gemini::GeminiGenerateContentRequest;
use serde_json::Value;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

use super::api::AntigravityApi;

#[derive(Debug, Clone)]
pub struct AntigravityContext {
    pub model: String,
    pub stream: bool,
    pub path: String,
    pub model_mask: u64,
}

pub struct AntigravityClient {
    client: reqwest::Client,
    retry_policy: ExponentialBuilder,
}

impl AntigravityClient {
    pub fn new(cfg: &AntigravityResolvedConfig, client: reqwest::Client) -> Self {
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

    pub async fn call_antigravity(
        &self,
        handle: &AntigravityActorHandle,
        ctx: &AntigravityContext,
        body: &GeminiGenerateContentRequest,
    ) -> Result<reqwest::Response, PolluxError> {
        let handle = handle.clone();
        let client = self.client.clone();
        let stream = ctx.stream;
        let model = ctx.model.clone();
        let model_mask = ctx.model_mask;
        let retry_policy_inner = self.retry_policy;
        let path = ctx.path.clone();
        let gemini_request = body.clone();

        let op = {
            let gemini_request = gemini_request.clone();
            move || {
                let handle = handle.clone();
                let client = client.clone();
                let gemini_request = gemini_request.clone();
                let model = model.clone();
                let path = path.clone();
                async move {
                    let start = Instant::now();
                    let assigned = handle
                        .get_credential(model_mask)
                        .await?
                        .ok_or(PolluxError::NoAvailableCredential)?;

                    let actor_took = start.elapsed();
                    info!(
                        channel = "antigravity",
                        lease.id = assigned.id,
                        lease.waited_us = actor_took.as_micros() as u64,
                        req.model = %model,
                        req.stream = stream,
                        req.path = %path,
                        "[Antigravity] [ID: {}] [{:?}] Post -> {}",
                        assigned.id,
                        actor_took,
                        model
                    );

                    let mut request = gemini_request.clone();
                    request
                        .extra
                        .entry("sessionId".to_string())
                        .or_insert_with(|| Value::String(AntigravityApi::generate_session_id()));

                    let payload = AntigravityRequestBody::from((
                        request,
                        AntigravityRequestMeta {
                            project: assigned.project_id.clone(),
                            request_id: AntigravityApi::generate_request_id(),
                            model: model.clone(),
                        },
                    ));

                    let resp = AntigravityApi::try_post(
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
                            |_json: GeminiCliErrorBody| PolluxError::UpstreamStatus(status),
                            |status, _body| PolluxError::UpstreamStatus(status),
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

                        warn!(
                            lease_id = assigned.id,
                            model = %model,
                            status = %status,
                            action = ?action,
                            "[Antigravity] Upstream error"
                        );

                        return Err(final_error);
                    }
                    Ok(resp)
                }
            }
        };

        op.retry(&self.retry_policy)
            .when(|err: &PolluxError| err.is_retryable())
            .notify(|err, dur: Duration| {
                error!(
                    "[Antigravity] Upstream Error {} retry after {:?}",
                    err.to_string(),
                    dur
                );
            })
            .await
    }
}
