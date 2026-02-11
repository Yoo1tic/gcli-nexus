use crate::config::AntigravityResolvedConfig;
use crate::error::{GeminiCliErrorBody, IsRetryable, PolluxError};
use crate::providers::antigravity::AntigravityActorHandle;
use crate::providers::policy::classify_upstream_error;
use crate::providers::provider_endpoints::ProviderEndpoints;
use crate::providers::upstream_retry::post_json_with_retry;
use crate::utils::logging::with_pretty_json_debug;
use backon::{ExponentialBuilder, Retryable};
use chrono::Utc;
use pollux_schema::{
    antigravity::AntigravityRequestMeta, gemini::GeminiGenerateContentRequest,
    gemini::GenerationConfig,
};
use rand::Rng as _;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};
use url::Url;
use uuid::Uuid;

const REQUEST_ID_PREFIX: &str = "agent";
const SESSION_ID_MAX_EXCLUSIVE: i64 = 9_000_000_000_000_000_000;
const CLAUDE_THINKING_BUDGET: u32 = 8096;

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
    endpoints: ProviderEndpoints,
}

impl AntigravityClient {
    pub fn new(
        cfg: &AntigravityResolvedConfig,
        client: reqwest::Client,
        base_url: Option<Url>,
    ) -> Self {
        let retry_policy = ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(100))
            .with_max_delay(Duration::from_millis(300))
            .with_max_times(cfg.retry_max_times)
            .with_jitter();
        let endpoints = base_url
            .map(Self::endpoints_for_base)
            .unwrap_or_else(Self::default_endpoints);

        Self {
            client,
            retry_policy,
            endpoints,
        }
    }

    fn default_endpoints() -> ProviderEndpoints {
        Self::endpoints_for_base(
            Url::parse("https://daily-cloudcode-pa.googleapis.com")
                .expect("invalid fixed Antigravity base URL"),
        )
    }

    fn endpoints_for_base(base: Url) -> ProviderEndpoints {
        ProviderEndpoints::new(
            base,
            "/v1internal:streamGenerateContent",
            Some("alt=sse"),
            "/v1internal:generateContent",
            None,
        )
    }

    pub async fn call_antigravity(
        &self,
        handle: &AntigravityActorHandle,
        ctx: &AntigravityContext,
        body: &GeminiGenerateContentRequest,
    ) -> Result<reqwest::Response, PolluxError> {
        let handle = handle.clone();
        let client = self.client.clone();
        let endpoints = self.endpoints.clone();
        let stream = ctx.stream;
        let model = ctx.model.clone();
        let model_mask = ctx.model_mask;
        let path = ctx.path.clone();
        let gemini_request = body.clone();

        let op = {
            let gemini_request = gemini_request.clone();
            move || {
                let handle = handle.clone();
                let client = client.clone();
                let endpoints = endpoints.clone();
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

                    let mut payload = AntigravityRequestMeta {
                        project: assigned.project_id.clone(),
                        request_id: Self::generate_request_id(),
                        model: model.clone(),
                    }
                    .into_request(gemini_request.clone());

                    Self::apply_claude_thinking_defaults(model.as_str(), &mut payload.request);

                    payload.prepend_system_instruction(crate::config::CLAUDE_SYSTEM_PREAMBLE);

                    payload
                        .request
                        .extra
                        .entry("sessionId".to_string())
                        .or_insert_with(|| Value::String(Self::generate_session_id()));

                    with_pretty_json_debug(&payload, |pretty_payload| {
                        debug!(
                            channel = "antigravity",
                            lease.id = assigned.id,
                            req.model = %model,
                            req.stream = stream,
                            req.path = %path,
                            body = %pretty_payload,
                            "[Antigravity] Prepared upstream payload"
                        );
                    });

                    let resp = post_json_with_retry(
                        "Antigravity",
                        &client,
                        endpoints.select(stream),
                        Some(Self::headers(assigned.access_token.as_str())),
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

    fn headers(access_token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {access_token}"))
                .expect("invalid fixed auth header value"),
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("antigravity/1.16.5 linux/amd64"),
        );
        headers
    }

    fn request_id_from_parts(timestamp_ms: i64, request_uuid: Uuid) -> String {
        format!("{REQUEST_ID_PREFIX}/{timestamp_ms}/{request_uuid}")
    }

    fn generate_request_id() -> String {
        Self::request_id_from_parts(Utc::now().timestamp_millis(), Uuid::new_v4())
    }

    fn session_id_from_int(value: i64) -> String {
        format!("-{value}")
    }

    fn generate_session_id() -> String {
        let value = rand::rng().random_range(0..SESSION_ID_MAX_EXCLUSIVE);
        Self::session_id_from_int(value)
    }

    fn apply_claude_thinking_defaults(model: &str, request: &mut GeminiGenerateContentRequest) {
        if !model.starts_with("claude") {
            return;
        }

        let gen_config = request
            .generation_config
            .get_or_insert_with(GenerationConfig::default);

        if gen_config.thinking_config.is_none() {
            gen_config.thinking_config = Some(json!({
                "includeThoughts": true,
                "thinkingBudget": CLAUDE_THINKING_BUDGET,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_id_uses_agent_timestamp_uuid_shape() {
        let id = AntigravityClient::request_id_from_parts(
            1234,
            Uuid::parse_str("00000000-0000-4000-8000-000000000000").unwrap(),
        );
        assert_eq!(id, "agent/1234/00000000-0000-4000-8000-000000000000");
    }

    #[test]
    fn endpoints_use_expected_literals() {
        let endpoints = AntigravityClient::default_endpoints();
        assert_eq!(
            endpoints.select(false).as_str(),
            "https://daily-cloudcode-pa.googleapis.com/v1internal:generateContent"
        );
        assert_eq!(
            endpoints.select(true).as_str(),
            "https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn session_id_is_negative_decimal_string() {
        assert_eq!(AntigravityClient::session_id_from_int(42), "-42");
        assert_eq!(AntigravityClient::session_id_from_int(0), "-0");
    }

    #[test]
    fn claude_requests_get_default_thinking_config_when_missing() {
        let mut request: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .expect("request must parse");

        AntigravityClient::apply_claude_thinking_defaults(
            "claude-sonnet-4-5-thinking",
            &mut request,
        );

        assert_eq!(
            request
                .generation_config
                .as_ref()
                .and_then(|cfg| cfg.thinking_config.as_ref()),
            Some(&json!({
                "includeThoughts": true,
                "thinkingBudget": CLAUDE_THINKING_BUDGET,
            }))
        );
    }

    #[test]
    fn claude_requests_keep_existing_thinking_config() {
        let mut request: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}],
            "generationConfig": {
                "thinkingConfig": {
                    "includeThoughts": false,
                    "thinkingBudget": 2048
                }
            }
        }))
        .expect("request must parse");

        AntigravityClient::apply_claude_thinking_defaults(
            "claude-sonnet-4-5-thinking",
            &mut request,
        );

        assert_eq!(
            request
                .generation_config
                .as_ref()
                .and_then(|cfg| cfg.thinking_config.as_ref()),
            Some(&json!({
                "includeThoughts": false,
                "thinkingBudget": 2048
            }))
        );
    }

    #[test]
    fn non_claude_requests_do_not_get_thinking_config_default() {
        let request: GeminiGenerateContentRequest = serde_json::from_value(json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .expect("request must parse");

        let model = "gemini-2.5-pro";
        let mut request = request;
        AntigravityClient::apply_claude_thinking_defaults(model, &mut request);

        assert!(request.generation_config.is_none());
    }
}
